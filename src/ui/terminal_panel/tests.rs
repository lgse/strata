// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{
    CancellationProgress, ChildExit, SessionKind, SessionLifecycle, SessionState, SourcePalette,
    SpawnCompletion, SpawnProgress, ThemeTokens, agent_argv, ansi_palette, exit_notice,
};

const ANYWHERE: &str = "/tmp";

fn tokens() -> ThemeTokens {
    ThemeTokens {
        name: "Probe".to_string(),
        background: "#101014".to_string(),
        surface: "#17171d".to_string(),
        text: "#e6e6ea".to_string(),
        accent: "#4f8cff".to_string(),
        danger: "#ff5f5f".to_string(),
        muted: "#6a6a76".to_string(),
        highlight: "#2a2a35".to_string(),
        border: "#31313c".to_string(),
        dim_text: "#9a9aa6".to_string(),
        syntax_keyword: None,
        syntax_string: None,
        syntax_constant: None,
        syntax_type: None,
        syntax_preprocessor: None,
    }
}

fn palette() -> SourcePalette {
    SourcePalette {
        statement: "#c77dff".to_string(),
        string: "#5fd97f".to_string(),
        constant: "#ffc861".to_string(),
        type_color: "#4fd6d6".to_string(),
        preprocessor: "#ff9f6b".to_string(),
    }
}

#[test]
fn theme_colors_fill_sixteen_distinguishable_ansi_slots() {
    let tokens = tokens();
    let colors = ansi_palette(&tokens, &palette());

    assert_eq!(colors.len(), 16);
    assert_eq!(colors[0], tokens.background);
    assert_eq!(colors[1], tokens.danger);
    assert_eq!(colors[4], tokens.accent);
    assert_eq!(colors[7], tokens.text);

    // Collapsing the chromatic slots would make diffs and ls output unreadable.
    let chromatic: HashSet<&String> = colors[1..7].iter().collect();
    assert_eq!(chromatic.len(), 6, "{colors:?}");

    for slot in 1..7 {
        assert_ne!(colors[slot], colors[slot + 8], "bright slot {slot} matches");
    }
}

#[test]
fn agent_arguments_keep_each_selected_path_separate() {
    let paths = [
        PathBuf::from("/tmp/two words.txt"),
        PathBuf::from("/tmp/it's here.txt"),
        PathBuf::from("/tmp/; rm -rf ~"),
    ];

    let argv = agent_argv("sh -c 'exit 0'", Path::new(ANYWHERE), &paths).expect("configured agent");

    assert!(
        Path::new(&argv[0]).is_absolute() && argv[0].ends_with("/sh"),
        "{argv:?}"
    );
    assert_eq!(argv[1..3], ["-c", "exit 0"]);
    assert_eq!(
        argv[3..],
        [
            "/tmp/two words.txt",
            "/tmp/it's here.txt",
            "/tmp/; rm -rf ~"
        ]
    );
}

#[test]
fn an_unconfigured_or_missing_agent_is_reported_not_launched() {
    assert!(agent_argv("", Path::new(ANYWHERE), &[]).is_err());
    assert!(agent_argv("   ", Path::new(ANYWHERE), &[]).is_err());

    let missing = agent_argv("strata-agent-that-does-not-exist", Path::new(ANYWHERE), &[])
        .expect_err("missing agent");
    assert!(
        missing.contains("strata-agent-that-does-not-exist"),
        "{missing}"
    );
    assert!(missing.contains("PATH"), "{missing}");

    let unparsable = agent_argv("agent 'unterminated", Path::new(ANYWHERE), &[])
        .expect_err("unparsable command");
    assert!(unparsable.contains("could not be read"), "{unparsable}");
}

#[test]
fn an_agent_that_exits_reports_how_it_ended() {
    assert!(exit_notice(0).contains("exited with status 0"));
    assert!(exit_notice(127 << 8).contains("exited with status 127"));
    assert!(exit_notice(2 << 8).contains("exited with status 2"));
    assert!(exit_notice(9).contains("terminated by signal 9"));
    assert!(exit_notice(15).contains("terminated by signal 15"));
    assert!(exit_notice(0).starts_with("\r\n"), "{:?}", exit_notice(0));
}

#[test]
fn a_placeholder_decides_where_selected_paths_land() {
    let paths = [PathBuf::from("/demo/a.rs"), PathBuf::from("/demo/b.rs")];

    let argv = agent_argv("sh {}", Path::new(ANYWHERE), &paths).expect("configured agent");
    assert_eq!(argv[1..], ["/demo/a.rs", "/demo/b.rs"]);

    let argv =
        agent_argv("sh 'review {} please'", Path::new(ANYWHERE), &paths).expect("configured agent");
    assert_eq!(argv[1..], ["review /demo/a.rs /demo/b.rs please"]);

    let argv =
        agent_argv("sh --flag {} --last", Path::new(ANYWHERE), &paths).expect("configured agent");
    assert_eq!(argv[1..], ["--flag", "/demo/a.rs", "/demo/b.rs", "--last"]);

    let argv = agent_argv("sh --flag", Path::new(ANYWHERE), &paths).expect("configured agent");
    assert_eq!(argv[1..], ["--flag", "/demo/a.rs", "/demo/b.rs"]);

    let argv = agent_argv("sh {} --last", Path::new(ANYWHERE), &[]).expect("configured agent");
    assert_eq!(argv[1..], ["--last"]);
    let argv = agent_argv("sh 'review {}'", Path::new(ANYWHERE), &[]).expect("configured agent");
    assert_eq!(argv[1..], ["review "]);
}

#[test]
fn a_path_that_cannot_be_passed_exactly_stops_the_launch() {
    use std::os::unix::ffi::OsStrExt;

    let mangled = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/bad-\xff.txt"));
    let error = agent_argv("sh", Path::new(ANYWHERE), &[mangled]).expect_err("lossy path");

    assert!(error.contains("without changing it"), "{error}");
}

#[test]
fn a_working_directory_that_cannot_be_passed_exactly_stops_the_launch() {
    use std::os::unix::ffi::OsStrExt;

    let directory = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/cwd-\xff"));
    let error = agent_argv("sh", &directory, &[]).expect_err("lossy working directory");

    assert!(error.contains("without changing it"), "{error}");
}

#[test]
fn a_relative_command_resolves_against_the_folder_the_agent_runs_in() {
    let directory = tempfile::tempdir().expect("agent directory fixture");
    let program = directory.path().join("agent-probe");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").expect("fixture program");
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("fixture permissions");

    let argv = agent_argv("./agent-probe", directory.path(), &[]).expect("relative agent");
    assert_eq!(argv, [program.to_str().expect("fixture path")]);

    let elsewhere = tempfile::tempdir().expect("other directory fixture");
    let error =
        agent_argv("./agent-probe", elsewhere.path(), &[]).expect_err("missing relative agent");
    assert!(error.contains("not an executable file"), "{error}");

    let absolute = program.to_str().expect("fixture path");
    assert_eq!(
        agent_argv(absolute, elsewhere.path(), &[]).expect("absolute agent"),
        [absolute]
    );
}

#[test]
fn the_panel_releases_its_state_when_dropped() {
    crate::test_support::gtk_test(
        "ui::terminal_panel::tests::the_panel_releases_its_state_when_dropped",
        || {
            let preferences = super::ThemeManager::shared();
            let panel = super::TerminalPanel::new(&preferences, std::rc::Rc::new(|| None));
            let state = std::rc::Rc::downgrade(&panel.state);

            drop(panel);

            // Callbacks the panel installs on its own widgets must not own it,
            // or a closed window keeps its terminal and child alive.
            assert!(
                state.upgrade().is_none(),
                "the panel is still held by {} reference(s)",
                state.strong_count()
            );
        },
    );
}

#[test]
fn pending_spawn_cannot_be_restarted_by_a_second_reveal() {
    let mut lifecycle = SessionLifecycle::default();

    let generation = lifecycle
        .request_spawn()
        .expect("first reveal starts a spawn");

    assert_eq!(lifecycle.request_spawn(), None);
    assert!(matches!(
        lifecycle.state(),
        SessionState::Spawning {
            generation: active,
            progress: SpawnProgress::Pending,
            ..
        } if *active == generation
    ));
}

#[test]
fn cancelling_pending_spawn_terminates_a_late_child_before_draining_it() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle.request_spawn().expect("spawn generation");

    assert_eq!(lifecycle.cancel_pending_spawn(), Some(generation));
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(41)),
        SpawnCompletion::Terminate(41)
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Cancelling {
            generation: active,
            progress: CancellationProgress::Child(41),
            ..
        } if *active == generation
    ));
    assert_eq!(lifecycle.child_exited(), ChildExit::BecameIdle);
}

#[test]
fn stopping_session_blocks_a_new_spawn_until_exit_is_drained() {
    let mut lifecycle = SessionLifecycle::default();
    let first = lifecycle.request_spawn().expect("first spawn generation");
    assert_eq!(
        lifecycle.complete_spawn(first, Ok(11)),
        SpawnCompletion::Running
    );

    assert_eq!(lifecycle.stop_running(), Some(11));
    assert_eq!(lifecycle.request_spawn(), None);
    assert_eq!(lifecycle.child_exited(), ChildExit::BecameIdle);
    assert!(lifecycle.request_spawn().is_some());
}

#[test]
fn stale_spawn_callback_cannot_replace_a_newer_generation() {
    let mut lifecycle = SessionLifecycle::default();
    let stale = lifecycle.request_spawn().expect("stale generation");
    assert_eq!(lifecycle.cancel_pending_spawn(), Some(stale));
    assert_eq!(
        lifecycle.complete_spawn(stale, Err(())),
        SpawnCompletion::BecomeIdle
    );
    let current = lifecycle.request_spawn().expect("current generation");

    assert_eq!(
        lifecycle.complete_spawn(stale, Ok(73)),
        SpawnCompletion::Stale(Some(73))
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Spawning {
            generation: active,
            progress: SpawnProgress::Pending,
            ..
        } if *active == current
    ));
}

#[test]
fn stale_spawn_failure_cannot_clear_a_newer_generation() {
    let mut lifecycle = SessionLifecycle::default();
    let stale = lifecycle.request_spawn().expect("stale generation");
    lifecycle.cancel_pending_spawn();
    assert_eq!(
        lifecycle.complete_spawn(stale, Err(())),
        SpawnCompletion::BecomeIdle
    );
    let current = lifecycle.request_spawn().expect("current generation");

    assert_eq!(
        lifecycle.complete_spawn(stale, Err(())),
        SpawnCompletion::Stale(None)
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Spawning {
            generation: active,
            progress: SpawnProgress::Pending,
            ..
        } if *active == current
    ));
}

#[test]
fn current_spawn_failure_returns_to_idle_and_allows_a_retry() {
    let mut lifecycle = SessionLifecycle::default();
    let failed = lifecycle.request_spawn().expect("failed generation");

    assert_eq!(
        lifecycle.complete_spawn(failed, Err(())),
        SpawnCompletion::BecomeIdle
    );
    assert!(lifecycle.request_spawn().is_some());
}

#[test]
fn cancelling_spawn_without_a_child_returns_to_idle_on_failure() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle.request_spawn().expect("spawn generation");
    lifecycle.cancel_pending_spawn();

    assert_eq!(
        lifecycle.complete_spawn(generation, Err(())),
        SpawnCompletion::BecomeIdle
    );
    assert!(matches!(lifecycle.state(), SessionState::Idle));
    assert!(lifecycle.request_spawn().is_some());
}

#[test]
fn exit_before_spawn_callback_does_not_deadlock_cancellation() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle.request_spawn().expect("spawn generation");
    lifecycle.cancel_pending_spawn();

    assert_eq!(lifecycle.child_exited(), ChildExit::Recorded);
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(59)),
        SpawnCompletion::TerminateAndBecomeIdle(59)
    );
    assert!(matches!(lifecycle.state(), SessionState::Idle));
}

#[test]
fn early_natural_shell_exit_returns_to_idle_after_spawn_callback() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Shell)
        .expect("shell spawn generation");

    assert_eq!(
        lifecycle.child_exited_with_status(7 << 8),
        ChildExit::Recorded
    );
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(59)),
        SpawnCompletion::TerminateAndBecomeIdle(59)
    );
    assert!(matches!(lifecycle.state(), SessionState::Idle));
}

#[test]
fn natural_shell_exit_returns_a_running_session_to_idle() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle.request_spawn().expect("spawn generation");
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(17)),
        SpawnCompletion::Running
    );

    assert_eq!(lifecycle.child_exited(), ChildExit::BecameIdle);
    assert!(matches!(lifecycle.state(), SessionState::Idle));
}

#[test]
fn hiding_and_showing_running_session_keeps_the_same_generation() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle.request_spawn().expect("spawn generation");
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(23)),
        SpawnCompletion::Running
    );

    assert_eq!(lifecycle.request_spawn(), None);
    assert!(matches!(
        lifecycle.state(),
        SessionState::Running {
            generation: active,
            pid: 23,
            ..
        } if *active == generation
    ));
}

#[test]
fn natural_agent_exit_retains_output_until_explicit_discard() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Agent)
        .expect("agent spawn generation");
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(31)),
        SpawnCompletion::Running
    );

    assert_eq!(
        lifecycle.child_exited_with_status(7 << 8),
        ChildExit::AgentCompleted(7 << 8)
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::ExitedAgentOutput { status } if *status == 7 << 8
    ));
    assert_eq!(lifecycle.request_spawn(), None);

    lifecycle.discard_completed_output();
    assert!(matches!(lifecycle.state(), SessionState::Idle));
    assert!(lifecycle.request_spawn().is_some());
}

#[test]
fn early_agent_exit_before_spawn_callback_retains_output_and_status() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Agent)
        .expect("agent spawn generation");
    let status = 7 << 8;

    assert_eq!(
        lifecycle.child_exited_with_status(status),
        ChildExit::Recorded
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Spawning {
            generation: active,
            kind: SessionKind::Agent,
            progress: SpawnProgress::ExitSeen(actual),
        } if *active == generation && *actual == status
    ));

    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(31)),
        SpawnCompletion::AgentCompleted(status)
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::ExitedAgentOutput { status: actual } if *actual == status
    ));
    assert_eq!(lifecycle.request_spawn(), None);

    lifecycle.discard_completed_output();
    assert!(matches!(lifecycle.state(), SessionState::Idle));
    assert!(lifecycle.request_spawn().is_some());
}

#[test]
fn closing_a_running_agent_drains_the_same_generic_stopping_state() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Agent)
        .expect("agent spawn generation");
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(37)),
        SpawnCompletion::Running
    );

    assert_eq!(lifecycle.stop_running(), Some(37));
    assert!(matches!(
        lifecycle.state(),
        SessionState::Stopping {
            kind: SessionKind::Agent,
            ..
        }
    ));
    assert_eq!(lifecycle.child_exited(), ChildExit::BecameIdle);
    assert!(matches!(lifecycle.state(), SessionState::Idle));
}

#[test]
fn agent_pending_spawn_uses_generic_cancellation_and_late_child_drain() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Agent)
        .expect("agent spawn generation");

    assert_eq!(lifecycle.cancel_pending_spawn(), Some(generation));
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(43)),
        SpawnCompletion::Terminate(43)
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Cancelling {
            kind: SessionKind::Agent,
            progress: CancellationProgress::Child(43),
            ..
        }
    ));
    assert_eq!(lifecycle.child_exited(), ChildExit::BecameIdle);
}

#[test]
fn cancelled_agent_exit_seen_before_callback_never_retains_output() {
    let mut lifecycle = SessionLifecycle::default();
    let generation = lifecycle
        .request_spawn_kind(SessionKind::Agent)
        .expect("agent spawn generation");
    let status = 7 << 8;

    assert_eq!(lifecycle.cancel_pending_spawn(), Some(generation));
    assert_eq!(
        lifecycle.child_exited_with_status(status),
        ChildExit::Recorded
    );
    assert!(matches!(
        lifecycle.state(),
        SessionState::Cancelling {
            kind: SessionKind::Agent,
            progress: CancellationProgress::Waiting(SpawnProgress::ExitSeen(actual)),
            ..
        } if *actual == status
    ));
    assert_eq!(
        lifecycle.complete_spawn(generation, Ok(43)),
        SpawnCompletion::TerminateAndBecomeIdle(43)
    );
    assert!(matches!(lifecycle.state(), SessionState::Idle));
}

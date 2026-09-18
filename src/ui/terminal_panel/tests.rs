// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use super::{
    CancellationProgress, ChildExit, SessionLifecycle, SessionState, SourcePalette,
    SpawnCompletion, SpawnProgress, ThemeTokens, ansi_palette,
};

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
            progress: SpawnProgress::Pending
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
            progress: CancellationProgress::Child(41)
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
            progress: SpawnProgress::Pending
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
            progress: SpawnProgress::Pending
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
            pid: 23
        } if *active == generation
    ));
}

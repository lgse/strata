use rune::Tx;
use strata_kit::icons::IconName;

use super::jobs::{self, JobRow, JobStatus, JobsWords, strip_terminal_sequences};
use super::*;

/// Ticks `dock` from wake to wake until `limit` seconds after `start`, as a resting window
/// polls: only at the wakes it asked for. Returns the clock it ended at, and whether any tick
/// changed something.
fn wake_to_wake(dock: &mut Dock, start: f64, limit: f64) -> f64 {
    let mut now = start;
    for _ in 0..1000 {
        let mut tx = Tx::default();
        dock.tick(now, &mut tx);
        let Some(next) = dock.asked_wake() else {
            break;
        };
        if now + next > start + limit {
            break;
        }
        now += next.max(0.001);
    }
    now
}

fn copy(id: OpId) -> OpSpec {
    let mut spec = OpSpec::new(IconName::Copy, "Copying items", "photos");
    spec.transfer = true;
    spec.destination = Some("→ Pictures".into());
    let _ = id;
    spec
}

fn transfer(bytes: u64, total: u64, files: usize, of: usize) -> Sample {
    Sample::new(Progress::Transfer(Transfer {
        completed_items: files,
        total_items: of,
        completed_files: files,
        total_files: Some(of),
        bytes,
        total_bytes: Some(total),
    }))
}

#[test]
fn sizes_rates_and_durations_read_as_strata_writes_them() {
    assert_eq!(format::size(999), "999 B");
    assert_eq!(format::size(1_500), "1.5 kB");
    assert_eq!(format::size(64_000_000), "64 MB");
    assert_eq!(
        format::size(999_960),
        "1 MB",
        "a value that rounds to 1000 moves up"
    );
    assert_eq!(format::size(1_234_000_000_000_000), "1,234 TB");
    assert_eq!(format::rate(1_500_000), "1.5 MB/s");
    assert_eq!(format::duration(45), "45s");
    assert_eq!(format::duration(125), "2m 5s");
    assert_eq!(format::duration(3_780), "1h 3m");
    assert_eq!(format::integer(270_325), "270,325");
    assert_eq!(
        format::whole_percent(0.004, true),
        1,
        "moving bytes show 1 %"
    );
    assert_eq!(format::whole_percent(0.004, false), 0);
    assert_eq!(format::whole_percent(0.999, true), 99, "rounded down");
    assert_eq!(format::seconds_left(Some(1_000.0), Some(1)), Some(1));
    assert_eq!(format::seconds_left(Some(1_000.0), Some(2_500)), Some(3));
    assert_eq!(format::seconds_left(None, Some(10)), None);
    assert_eq!(format::seconds_left(Some(10.0), Some(0)), None);
}

#[test]
fn the_rate_measures_after_a_quarter_second_and_blends_new_measurements() {
    let mut m = RateMeter::new();
    m.sample(1_000, 0.0);
    assert_eq!(m.rate(), None, "one sample only starts the clock");
    m.sample(2_000, 0.1);
    assert_eq!(m.rate(), None, "a sample within 250 ms measures nothing");
    m.sample(11_000, 1.0);
    assert_eq!(m.rate(), Some(10_000.0));
    m.sample(31_000, 2.0);
    assert_eq!(m.rate(), Some(10_000.0 * 0.6 + 20_000.0 * 0.4));
    let before = m.rate();
    m.sample(31_000, 9.0);
    assert_eq!(m.rate(), before, "a stalled copy keeps its last rate");
}

#[test]
fn a_countdown_pauses_keeps_its_time_and_fades_before_it_ends() {
    let mut c = Countdown::new(5.0, true);
    let s = c.tick(10.0);
    assert!(
        s.restarted && s.wake.is_some(),
        "the first tick stamps the start"
    );
    c.tick(12.0);
    assert!((c.left() - 3.0).abs() < 1e-9);
    c.set_hovered(true);
    c.tick(12.5);
    assert!((c.left() - 2.5).abs() < 1e-9);
    let s = c.tick(20.0);
    assert!(s.wake.is_none(), "a paused countdown asks for no wake");
    assert!((c.left() - 2.5).abs() < 1e-9, "paused, it keeps its time");
    c.set_hovered(false);
    assert!(c.tick(21.0).restarted, "leaving resumes");
    let s = c.tick(23.5);
    assert!(s.leaving && !s.done);
    assert!(c.tick(23.5 + FADE).done);
}

#[test]
fn a_pause_during_the_fade_stops_it_and_resuming_fades_again() {
    let mut c = Countdown::new(1.0, true);
    c.tick(0.0);
    assert!(c.tick(1.0).leaving);
    c.toggle_pin();
    c.tick(1.05);
    assert!(!c.leaving(), "the pin stops the fade");
    c.toggle_pin();
    let s = c.tick(3.0);
    assert!(s.leaving, "no time is left, so it fades again at once");
    assert!(c.tick(3.0 + FADE).done);
}

#[test]
fn with_animations_off_a_countdown_ends_without_a_fade() {
    let mut c = Countdown::new(1.0, false);
    c.tick(0.0);
    let s = c.tick(1.0);
    assert!(s.leaving && s.done);
}

#[test]
fn a_transfer_card_shows_percentage_speed_and_time_left_and_counts_files() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.progress(1, transfer(0, 40_000_000, 0, 10), 0.0, &mut tx);
    let op = dock.op(1).unwrap();
    assert_eq!(&*op.live.meta.get(), "Calculating speed…");
    assert_eq!(&*op.live.percent.get(), "0%");
    dock.progress(1, transfer(4_000_000, 40_000_000, 1, 10), 1.0, &mut tx);
    dock.progress(
        1,
        transfer(12_000_000, 40_000_000, 3, 10).current("b.jpg"),
        2.0,
        &mut tx,
    );
    let op = dock.op(1).unwrap();
    // Zero bytes start nothing; 4 MB at 1 s starts the clock and 12 MB at 2 s measures
    // 8 MB/s; 28 MB left is 3.5 s, rounded up.
    assert_eq!(&*op.live.meta.get(), "8 MB/s · 4s left");
    assert_eq!(&*op.live.percent.get(), "30%");
    assert_eq!(&*op.live.count.get(), "3/10");
    assert_eq!(&*op.live.bytes.get(), "12 MB / 40 MB");
    assert_eq!(
        op.shown.info, "b.jpg",
        "the current file replaces the info line"
    );
    assert_eq!(op.shown.value.as_deref(), Some("30%"));
    assert_eq!(
        op.shown.items_line.as_deref(),
        Some("3 of 10 files · b.jpg")
    );
}

#[test]
fn transfer_words_follow_what_is_known() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    let unknown = Sample::new(Progress::Transfer(Transfer::default()));
    dock.progress(1, unknown, 0.0, &mut tx);
    let op = dock.op(1).unwrap();
    assert_eq!(op.shown.fraction, None, "no total yet: it pulses");
    assert_eq!(&*op.live.status.get(), "Preparing…");
    assert_eq!(&*op.live.percent.get(), "…");
    assert_eq!(&*op.live.meta.get(), "Preparing items…");
    let moving = Sample::new(Progress::Transfer(Transfer {
        total_items: 2,
        bytes: 10,
        ..Transfer::default()
    }));
    dock.progress(1, moving, 1.0, &mut tx);
    assert_eq!(&*dock.op(1).unwrap().live.status.get(), "Transferring…");
    let empty_files = Sample::new(Progress::Transfer(Transfer {
        completed_items: 1,
        total_items: 4,
        total_bytes: Some(0),
        ..Transfer::default()
    }));
    dock.progress(1, empty_files, 2.0, &mut tx);
    let op = dock.op(1).unwrap();
    assert_eq!(
        op.shown.fraction,
        Some(0.25),
        "a tree of empty files counts items"
    );
    let flushing = transfer(40, 40, 1, 1).busy("Writing to device…");
    dock.progress(1, flushing, 3.0, &mut tx);
    let op = dock.op(1).unwrap();
    assert_eq!(op.shown.fraction, None, "the flush pulses");
    assert_eq!(&*op.live.meta.get(), "Writing to device…");
}

#[test]
fn a_one_item_delete_counts_and_pulses_and_a_larger_one_shows_its_percentage() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(
        1,
        OpSpec::new(IconName::Trash, "Deleting items", "big"),
        &mut tx,
    );
    dock.progress(
        1,
        Sample::new(Progress::Count(270_325)).status("270,325 items deleted"),
        0.0,
        &mut tx,
    );
    let op = dock.op(1).unwrap();
    assert_eq!(&*op.live.meta.get(), "270,325 items deleted");
    assert_eq!(&*op.live.percent.get(), "…");
    assert_eq!(op.shown.fraction, None);
    dock.start(
        2,
        OpSpec::new(IconName::Trash, "Deleting items", "a"),
        &mut tx,
    );
    dock.progress(
        2,
        Sample::new(Progress::Items { done: 1, total: 4 }),
        0.0,
        &mut tx,
    );
    let op = dock.op(2).unwrap();
    assert_eq!(&*op.live.meta.get(), "", "a delete's left label is empty");
    assert_eq!(&*op.live.percent.get(), "25%");
}

#[test]
fn committed_text_lands_at_most_four_times_a_second_and_a_held_change_flushes_alone() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    assert!(dock.progress(1, transfer(1, 100_000, 0, 10), 0.0, &mut tx));
    let mut commits = 0;
    let mut now = 0.0;
    for i in 1..=60 {
        now = f64::from(i) / 60.0;
        let mut tx = Tx::default();
        let s =
            transfer(1 + i as u64 * 1_000, 100_000, i as usize / 6, 10).current(format!("f{i}"));
        if dock.progress(1, s, now, &mut tx) {
            commits += 1;
        }
    }
    assert!(commits <= 4, "{commits} commits in a second");
    assert_eq!(
        &*dock.op(1).unwrap().live.count.get(),
        "10/10",
        "live words follow every sample"
    );
    // A last change held back lands from its own wake, with no sample after it.
    let mut tx = Tx::default();
    let held = dock.progress(
        1,
        transfer(99_000, 100_000, 10, 10).current("last"),
        now + 0.01,
        &mut tx,
    );
    assert!(!held, "within the quarter second it is held");
    assert!(dock.asked_wake().is_some(), "and asks for a wake");
    let end = wake_to_wake(&mut dock, now + 0.01, 1.0);
    assert_eq!(dock.op(1).unwrap().shown.info, "last", "flushed at {end}");
}

#[test]
fn a_small_operation_waits_350_ms_and_never_shows_if_it_ends_first() {
    assert_eq!(reveal_after(Some(3)), REVEAL_DELAY);
    assert_eq!(reveal_after(Some(16)), 0.0);
    assert_eq!(reveal_after(None), 0.0, "an unknown count shows at once");
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    let mut spec = copy(1);
    spec.reveal_after = REVEAL_DELAY;
    dock.start(1, spec.clone(), &mut tx);
    dock.tick(0.0, &mut tx);
    assert!(dock.is_empty());
    dock.tick(0.3, &mut tx);
    assert!(dock.is_empty());
    assert!(
        !dock.complete(1, "Copy complete", &mut tx),
        "it ended before it showed"
    );
    assert!(dock.op(1).is_none() && dock.is_empty());
    dock.start(2, spec, &mut tx);
    let end = wake_to_wake(&mut dock, 1.0, 1.0);
    assert!(!dock.is_empty(), "it shows from its own wake, by {end}");
    assert!((end - 1.0 - REVEAL_DELAY).abs() < 1e-6);
}

#[test]
fn cancel_disables_itself_and_a_transfer_stalls_after_eight_seconds() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.start(
        2,
        OpSpec::new(IconName::Archive, "Compressing items", "a.zip"),
        &mut tx,
    );
    assert!(dock.cancel(1, &mut tx));
    assert!(!dock.cancel(1, &mut tx), "a second press does nothing");
    let op = dock.op(1).unwrap();
    assert_eq!(op.title(), "Cancelling transfer…");
    assert!(!op.shown.cancel_enabled);
    assert_eq!(op.shown.info, Words::default().still_writing);
    assert!(dock.op(2).unwrap().running(), "the other card goes on");
    assert!(dock.cancel_pending());
    assert!(dock.cancel(2, &mut tx));
    assert_eq!(dock.op(2).unwrap().title(), "Cancelling operation…");
    let end = wake_to_wake(&mut dock, 5.0, 20.0);
    assert!(
        (end - 13.0).abs() < 1e-6,
        "the stall lands at its wake: {end}"
    );
    assert_eq!(dock.op(1).unwrap().title(), "Device not responding");
    assert_eq!(
        dock.op(2).unwrap().title(),
        "Cancelling operation…",
        "only transfers stall"
    );
}

#[test]
fn a_completed_card_closes_after_five_seconds_from_wake_to_wake() {
    for animations in [true, false] {
        let mut dock = Dock::new();
        dock.set_animations(animations);
        let mut tx = Tx::default();
        dock.start(1, copy(1), &mut tx);
        dock.progress(1, transfer(10, 40, 1, 4), 0.0, &mut tx);
        assert!(dock.complete(1, "Copy complete", &mut tx));
        let op = dock.op(1).unwrap();
        assert_eq!(op.shown.icon, Some(IconName::Check));
        assert_eq!(op.shown.value.as_deref(), Some("100%"));
        assert_eq!(&*op.live.count.get(), "4/4");
        wake_to_wake(&mut dock, 1.0, 4.9);
        assert!(dock.op(1).is_some(), "still there at 4.9 s");
        let end = wake_to_wake(&mut dock, 1.0, 10.0);
        assert!(dock.op(1).is_none(), "gone (animations {animations})");
        let fade = if animations { FADE } else { 0.0 };
        assert!((end - 6.0 - fade).abs() < 1e-6, "ended at {end}");
    }
}

#[test]
fn hover_and_the_pin_hold_a_finished_card_and_its_words_say_so() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.complete(1, "Copy complete", &mut tx);
    dock.tick(0.0, &mut tx);
    dock.tick(1.2, &mut tx);
    let ended = |d: &Dock| d.op(1).unwrap().shown.ended.clone().unwrap();
    assert_eq!(ended(&dock).words, "closing in 4s");
    dock.hover(1, true, &mut tx);
    dock.tick(1.3, &mut tx);
    assert_eq!(ended(&dock).words, "paused");
    wake_to_wake(&mut dock, 1.3, 60.0);
    assert!(dock.op(1).is_some(), "hovered, it stays");
    dock.hover(1, false, &mut tx);
    dock.toggle_pin(1, &mut tx);
    dock.tick(61.0, &mut tx);
    assert_eq!(ended(&dock).words, "pinned");
    wake_to_wake(&mut dock, 61.0, 60.0);
    assert!(dock.op(1).is_some(), "pinned, it stays");
    dock.toggle_pin(1, &mut tx);
    let end = wake_to_wake(&mut dock, 130.0, 10.0);
    assert!(dock.op(1).is_none());
    assert!(
        (end - 130.0 - 3.7 - FADE).abs() < 1e-6,
        "the remaining 3.7 s: {end}"
    );
}

#[test]
fn a_notice_replaces_the_notices_showing_unless_it_joins_and_cards_stay() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.notice(Notice::new("Sent to Phone"), &mut tx);
    dock.notice(Notice::new("Shared").heading("Provider"), &mut tx);
    assert_eq!(dock.notices().count(), 1, "the first was replaced");
    dock.notice(Notice::new("2 items were renamed").joins(), &mut tx);
    assert_eq!(dock.notices().count(), 2, "a report joins");
    assert!(dock.op(1).is_some());
    dock.remove(1);
    wake_to_wake(&mut dock, 0.0, 3.9);
    assert!(!dock.is_empty());
    wake_to_wake(&mut dock, 0.0, 10.0);
    assert!(dock.is_empty(), "both expired and the dock is empty");
}

#[test]
fn a_cancelled_card_can_become_a_notice_that_closes_itself() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.cancel(1, &mut tx);
    assert!(dock.close_as_notice(1, "Operation cancelled", "3 items completed", 4.0, &mut tx));
    let op = dock.op(1).unwrap();
    assert_eq!(op.ended(), Some(EndKind::Notice));
    assert_eq!(op.shown.info, "3 items completed");
    let end = wake_to_wake(&mut dock, 0.0, 10.0);
    assert!(dock.op(1).is_none());
    assert!((end - 4.0 - FADE).abs() < 1e-6);
}

#[test]
fn an_untimed_cancel_stalls_eight_seconds_after_the_next_tick() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.tick(0.0, &mut tx);
    assert!(dock.cancel_untimed(1));
    assert!(!dock.cancel_untimed(1), "once");
    assert!(
        dock.wants_wake().is_some(),
        "the views mount a beacon until the next tick"
    );
    let end = wake_to_wake(&mut dock, 1.0, 20.0);
    assert!(dock.wants_wake().is_none());
    assert_eq!(dock.op(1).unwrap().title(), "Device not responding");
    assert!((end - 9.0).abs() < 1e-6, "{end}");
}

#[test]
fn jobs_words_follow_strata() {
    let w = JobsWords::default();
    let mut rows = vec![
        JobRow::new(1, "Resize", JobStatus::Running, 0.0),
        JobRow::new(2, "Resize", JobStatus::Running, 0.0),
        JobRow::new(3, "Batch rename", JobStatus::Queued, 0.0),
    ];
    assert_eq!(
        jobs::indicator(&rows, &w).as_deref(),
        Some("2 jobs running · 1 queued")
    );
    for r in &mut rows {
        r.status = JobStatus::Succeeded;
    }
    assert_eq!(
        jobs::indicator(&rows, &w).as_deref(),
        Some("3 jobs finished")
    );
    rows[0].status = JobStatus::Failed;
    assert_eq!(
        jobs::indicator(&rows, &w).as_deref(),
        Some("3 jobs finished · failures")
    );
    assert_eq!(jobs::indicator(&[], &w), None, "no jobs: no button");
    let cancelling = vec![JobRow::new(1, "Resize", JobStatus::Cancelling, 0.0)];
    assert_eq!(
        jobs::indicator(&cancelling, &w).as_deref(),
        Some("1 job running"),
        "a job cancelling still runs until it stops"
    );
    assert_eq!(jobs::per_item_fraction(2, 10, Some(0.5)), Some(0.25));
    assert_eq!(jobs::per_item_fraction(0, 0, None), None);
}

#[test]
fn terminal_sequences_are_stripped_from_output() {
    assert_eq!(
        strip_terminal_sequences("\u{1b}[1;32mok\u{1b}[0m done"),
        "ok done"
    );
    assert_eq!(
        strip_terminal_sequences("\u{1b}]0;title\u{7}a\tb\nc\r"),
        "a\tb\nc"
    );
    assert_eq!(strip_terminal_sequences("x\u{1b}]8;;url\u{1b}\\y"), "xy");
}

#[test]
fn the_jobs_clock_ticks_each_second_only_while_open_with_a_running_job() {
    let mut st = jobs::JobsState::new();
    let rows = vec![JobRow::new(1, "Resize", JobStatus::Running, 0.3)];
    let mut tx = Tx::default();
    assert!(!st.tick(1.0, &rows, &mut tx));
    assert!(st.due().is_none(), "closed: no wakes");
    st.open(&mut tx);
    assert!(st.tick(1.0, &rows, &mut tx), "opening refreshes the clock");
    let due = st.due().unwrap();
    assert!(
        (due - 1.3).abs() < 1e-6,
        "the next whole second of elapsed time: {due}"
    );
    assert!(!st.tick(1.1, &rows, &mut tx), "nothing moves before it");
    assert!(st.tick(due, &rows, &mut tx));
    st.close();
    st.tick(5.0, &rows, &mut tx);
    assert!(st.due().is_none());
}

#[test]
fn a_card_that_finishes_under_the_pointer_starts_its_countdown_paused() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.tick(0.0, &mut tx);
    // The pointer arrives while the copy runs and rests there: Rune sends no second enter.
    dock.hover(1, true, &mut tx);
    dock.progress(1, transfer(10, 40, 1, 4), 0.5, &mut tx);
    dock.complete(1, "Copy complete", &mut tx);
    wake_to_wake(&mut dock, 1.0, 30.0);
    let op = dock.op(1).expect("the hovered card stays");
    assert_eq!(op.shown.ended.as_ref().unwrap().words, "paused");
    dock.hover(1, false, &mut tx);
    let end = wake_to_wake(&mut dock, 40.0, 10.0);
    assert!(dock.op(1).is_none());
    assert!(
        (end - 40.0 - 5.0 - FADE).abs() < 1e-6,
        "the whole 5 s: {end}"
    );
}

#[test]
fn a_second_completion_is_ignored_as_strata_ignores_it() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    assert!(dock.complete(1, "Copy complete", &mut tx));
    dock.tick(0.0, &mut tx);
    dock.tick(3.0, &mut tx);
    assert!(
        dock.complete(1, "Copy complete again", &mut tx),
        "it still shows"
    );
    assert!(
        dock.close_as_notice(1, "Operation cancelled", "x", 4.0, &mut tx),
        "it still shows; a later notice doesn't replace it"
    );
    assert_eq!(dock.op(1).unwrap().title(), "Copy complete");
    let end = wake_to_wake(&mut dock, 3.0, 10.0);
    assert!(dock.op(1).is_none());
    assert!(
        (end - 5.0 - FADE).abs() < 1e-6,
        "the first countdown ran on: {end}"
    );
}

#[test]
fn a_resized_track_moves_the_fill_and_restarts_a_countdown_bar() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.progress(1, transfer(10, 40, 1, 4), 0.0, &mut tx);
    assert!(
        dock.set_track(1, 200.0, 4.0),
        "the first size needs the view"
    );
    assert_eq!(dock.op(1).unwrap().live.fill.get(), [0.0, 0.0, 50.0, 4.0]);
    assert!(
        !dock.set_track(1, 200.2, 4.0),
        "a move within half a point is no resize"
    );
    assert!(
        !dock.set_track(1, 220.0, 4.0),
        "a running card's fill is live: no commit"
    );
    assert_eq!(dock.op(1).unwrap().live.fill.get(), [0.0, 0.0, 55.0, 4.0]);
    dock.complete(1, "Copy complete", &mut tx);
    dock.tick(0.0, &mut tx);
    assert!(
        dock.set_track(1, 240.0, 4.0),
        "a counting card's bar restarts at the new width"
    );
}

#[test]
fn a_card_that_changes_shape_commits_at_once_inside_the_quarter_second() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    assert!(dock.progress(1, transfer(10, 4_000, 1, 4), 0.0, &mut tx));
    assert!(
        !dock.progress(1, transfer(20, 4_000, 1, 4), 0.05, &mut tx),
        "a change of numbers waits"
    );
    // The flush to the device pulses the bar: a change of shape lands at once.
    let busy = transfer(4_000, 4_000, 4, 4).busy("Writing to device…");
    assert!(dock.progress(1, busy, 0.1, &mut tx));
    assert_eq!(dock.op(1).unwrap().shown.fraction, None);
}

#[test]
fn a_sample_after_completion_leaves_the_finished_card_alone() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.start(1, copy(1), &mut tx);
    dock.progress(1, transfer(10, 40, 1, 4), 0.0, &mut tx);
    dock.complete(1, "Copy complete", &mut tx);
    let before = dock.op(1).unwrap().shown.clone();
    assert!(!dock.progress(1, transfer(20, 40, 2, 4).current("late.jpg"), 0.5, &mut tx));
    let op = dock.op(1).unwrap();
    assert_eq!(op.shown, before);
    assert_eq!(&*op.live.percent.get(), "100%");
    assert_eq!(&*op.live.count.get(), "4/4");
}

#[test]
fn a_notice_shows_for_its_own_time() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    dock.notice(
        Notice::new("Text size: 16px").status().shown_for(1.5),
        &mut tx,
    );
    let end = wake_to_wake(&mut dock, 0.0, 10.0);
    assert!(dock.is_empty());
    assert!((end - 1.5).abs() < 1e-6, "{end}");
}

#[test]
fn an_operation_in_the_dialog_leaves_the_dock_empty() {
    let mut dock = Dock::new();
    let mut tx = Tx::default();
    let mut spec = copy(1);
    spec.placement = Placement::Dialog;
    dock.start(1, spec, &mut tx);
    assert!(dock.op(1).is_some());
    assert!(dock.is_empty(), "the dialog shows it, not the dock");
}

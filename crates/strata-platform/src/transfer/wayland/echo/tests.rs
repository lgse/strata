use super::Echoes;

#[test]
fn a_write_with_no_selection_event_before_its_sync_was_dropped() {
    let mut e = Echoes::default();
    e.arm(7);
    assert!(e.done(7));
}

#[test]
fn a_write_echoed_before_its_sync_took() {
    let mut e = Echoes::default();
    e.arm(7);
    e.selection();
    assert!(!e.done(7));
}

#[test]
fn a_selection_event_from_before_a_write_doesnt_count_for_it() {
    let mut e = Echoes::default();
    e.arm(1);
    e.selection();
    e.arm(2);
    assert!(!e.done(1));
    assert!(e.done(2));
}

#[test]
fn a_sync_for_a_write_never_armed_says_nothing() {
    let mut e = Echoes::default();
    assert!(!e.done(3));
    e.arm(3);
    assert!(e.done(3));
    assert!(!e.done(3), "each sync answers once");
}

#[test]
fn with_two_writes_in_flight_one_echo_answers_only_the_first() {
    // The compositor handles each write, then its sync, in order: the first write's echo
    // arrives before its sync's answer, and says nothing about the second write.
    let mut e = Echoes::default();
    e.arm(1);
    e.arm(2);
    e.selection();
    assert!(!e.done(1), "the first took");
    assert!(e.done(2), "the second was dropped");
}

#[test]
fn a_second_event_before_the_first_sync_answers_no_later_write() {
    let mut e = Echoes::default();
    e.arm(1);
    e.arm(2);
    e.selection();
    e.selection();
    assert!(!e.done(1));
    assert!(e.done(2), "the second write had no echo of its own");
}

#[test]
fn a_dropped_write_is_remembered_and_one_that_took_isnt() {
    let mut e = Echoes::default();
    e.arm(1);
    e.done(1);
    e.arm(2);
    e.selection();
    e.done(2);
    assert!(e.was_dropped(1));
    assert!(!e.was_dropped(2));
    assert!(!e.was_dropped(3), "never written");
}

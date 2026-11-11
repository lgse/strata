use super::*;

fn folders(names: &[&str]) -> Vec<Candidate> {
    names
        .iter()
        .map(|n| Candidate::new(format!("{n}/"), format!("/home/demo/{n}/")))
        .collect()
}

/// A model showing `names` for `text`.
fn showing(text: &str, names: &[&str]) -> Completion {
    let mut c = Completion::default();
    let q = c.request(text);
    c.arrive(q.generation, folders(names), text).expect("fresh");
    c
}

#[test]
fn an_answer_for_another_generation_or_text_is_dropped() {
    let mut c = Completion::default();
    let first = c.request("/home/demo/n");
    let second = c.request("/home/demo/D");
    assert!(
        c.arrive(first.generation, folders(&["nas"]), "/home/demo/D")
            .is_none()
    );
    assert!(c.is_pending(), "the newer listing still waits");
    // The right generation, but the text moved on without a new request: dropped too.
    assert!(
        c.arrive(second.generation, folders(&["Desktop"]), "/home/demo/De")
            .is_none()
    );
    assert!(!c.is_open());
}

#[test]
fn the_previous_list_stays_while_the_next_is_listed() {
    let mut c = showing("/home/demo/D", &["Desktop", "Documents"]);
    c.press(Command::Down, true, "/home/demo/D");
    assert_eq!(c.highlighted(), Some(0));
    c.request("/home/demo/Do");
    assert!(c.is_open());
    assert_eq!(c.shown().len(), 2);
    assert_eq!(c.highlighted(), None, "a new listing clears the highlight");
}

#[test]
fn arrows_wrap_and_paging_moves_five() {
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    let mut c = showing("/home/demo/", &names);
    let t = "/home/demo/";
    c.press(Command::Step(-1), false, t);
    assert_eq!(
        c.highlighted(),
        Some(6),
        "Up from nothing lands on the last"
    );
    c.press(Command::Down, false, t);
    assert_eq!(
        c.highlighted(),
        Some(0),
        "Down past the end wraps to the first"
    );
    c.press(Command::Step(5), false, t);
    assert_eq!(c.highlighted(), Some(5));
    c.press(Command::Step(5), false, t);
    assert_eq!(c.highlighted(), Some(3), "Page Down wraps");
    c.press(Command::Step(-5), false, t);
    assert_eq!(c.highlighted(), Some(5), "Page Up wraps back");
}

#[test]
fn tab_takes_the_common_prefix_then_cycles() {
    let mut c = Completion::default();
    let q = c.request("/home/demo/D");
    let list = vec![
        Candidate::new("Documents/", "/home/demo/Documents/"),
        Candidate::new("Downloads/", "/home/demo/Downloads/"),
    ];
    c.arrive(q.generation, list, "/home/demo/D").unwrap();
    let (out, _) = c.press(Command::Complete, true, "/home/demo/D");
    assert_eq!(out.text.as_deref(), Some("/home/demo/Do"));
    assert!(!c.is_open(), "a completion closes the list");
    // Down reopens it with a fresh listing for the completed text.
    let (out, _) = c.press(Command::Down, true, "/home/demo/Do");
    let q = out.queries.first().expect("a new query").clone();
    assert_eq!(q.text, "/home/demo/Do");
    let list = vec![
        Candidate::new("Documents/", "/home/demo/Documents/"),
        Candidate::new("Downloads/", "/home/demo/Downloads/"),
    ];
    c.arrive(q.generation, list, "/home/demo/Do").unwrap();
    // Nothing longer to insert: Tab highlights the next suggestion.
    let (out, _) = c.press(Command::Complete, false, "/home/demo/Do");
    assert_eq!(out.text, None);
    assert_eq!(c.highlighted(), Some(0));
    // With one highlighted, Tab inserts it.
    let (out, _) = c.press(Command::Complete, false, "/home/demo/Do");
    assert_eq!(out.text.as_deref(), Some("/home/demo/Documents/"));
}

#[test]
fn tab_inserts_the_only_suggestion() {
    let mut c = showing("/home/demo/Mu", &["Music"]);
    let (out, _) = c.press(Command::Complete, false, "/home/demo/Mu");
    assert_eq!(out.text.as_deref(), Some("/home/demo/Music/"));
}

#[test]
fn keys_during_a_listing_wait_and_run_in_order() {
    let mut c = showing("/home/demo/", &["Desktop", "Documents", "Downloads"]);
    let q = c.request("/home/demo/D");
    // Down while the list shows waits, Enter after it waits too.
    assert_eq!(
        c.press(Command::Down, false, "/home/demo/D"),
        (Out::default(), false)
    );
    assert_eq!(
        c.press(Command::Activate, false, "/home/demo/D"),
        (Out::default(), false)
    );
    let out = c
        .arrive(
            q.generation,
            folders(&["Desktop", "Documents"]),
            "/home/demo/D",
        )
        .unwrap();
    // Down highlighted the first of the new list, and Enter took it.
    assert_eq!(out.submit.as_deref(), Some("/home/demo/Desktop/"));
    assert_eq!(out.text.as_deref(), Some("/home/demo/Desktop/"));
}

#[test]
fn enter_alone_during_a_listing_goes_on() {
    let mut c = showing("/home/demo/", &["Desktop"]);
    c.request("/home/demo/D");
    let (out, proceeds) = c.press(Command::Activate, false, "/home/demo/D");
    assert!(
        proceeds,
        "Enter with nothing waiting submits the typed text"
    );
    assert!(out.cancelled);
}

#[test]
fn tab_during_a_listing_waits_for_it() {
    let mut c = Completion::default();
    let q = c.request("/home/demo/Mu");
    assert!(!c.press(Command::Complete, true, "/home/demo/Mu").1);
    let out = c
        .arrive(q.generation, folders(&["Music"]), "/home/demo/Mu")
        .unwrap();
    assert_eq!(out.text.as_deref(), Some("/home/demo/Music/"));
}

#[test]
fn a_tab_that_relists_carries_the_rest_to_the_new_listing() {
    // Shown, then Tab completes, Down relists (the text changed): the Enter behind it moves to
    // the new listing.
    let mut c = showing("/home/demo/", &["Desktop", "Documents"]);
    let q = c.request("/home/demo/Mu");
    for cmd in [Command::Complete, Command::Down, Command::Activate] {
        c.press(cmd, false, "/home/demo/Mu");
    }
    let out = c
        .arrive(q.generation, folders(&["Music"]), "/home/demo/Mu")
        .unwrap();
    assert_eq!(out.text.as_deref(), Some("/home/demo/Music/"));
    let next = out.queries.last().expect("Down asked again").clone();
    assert_eq!(next.text, "/home/demo/Music/");
    assert_eq!(out.submit, None, "Enter waits for the new listing");
    // Down only reopened the list, so Enter, nothing highlighted, submits the text as it stands.
    let out = c
        .arrive(
            next.generation,
            folders(&["Music/Live"]),
            "/home/demo/Music/",
        )
        .unwrap();
    assert_eq!(out.submit.as_deref(), Some("/home/demo/Music/"));
}

#[test]
fn common_prefix_respects_character_boundaries() {
    let a = ["/home/café/", "/home/cafè/"];
    assert_eq!(longest_common_prefix(a.into_iter()), Some("/home/caf"));
    assert_eq!(longest_common_prefix(["x", "y"].into_iter()), None);
    assert_eq!(longest_common_prefix(std::iter::empty()), None);
}

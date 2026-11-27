// SPDX-License-Identifier: MIT

use std::sync::mpsc;

use strata_fs::services::{SearchCoverage, SearchEvent};

use super::drain;

fn results(query: &str, candidates: usize) -> SearchEvent {
    SearchEvent::Results {
        query: query.to_owned(),
        items: Vec::new(),
        indexing: false,
        coverage: SearchCoverage::default(),
        has_more: false,
        candidates,
    }
}

#[test]
fn a_poll_keeps_the_last_batch_for_the_query_typed_now() {
    let (sender, receiver) = mpsc::channel();
    for (query, candidates) in [("note", 1), ("old", 2), ("note", 3), ("", 4)] {
        sender.send(results(query, candidates)).unwrap();
    }
    let (latest, disconnected) = drain(&receiver, "note", false);
    assert_eq!(latest.map(|batch| batch.candidates), Some(3));
    assert!(!disconnected);
}

#[test]
fn a_poll_reads_eight_events_and_leaves_the_rest_for_the_next() {
    let (sender, receiver) = mpsc::channel();
    for candidates in 1..=10 {
        sender.send(results("note", candidates)).unwrap();
    }
    let (first, _) = drain(&receiver, "note", false);
    let (second, _) = drain(&receiver, "note", false);
    assert_eq!(first.map(|batch| batch.candidates), Some(8));
    assert_eq!(second.map(|batch| batch.candidates), Some(10));
}

#[test]
fn a_worker_that_has_gone_ends_the_polls_after_its_last_batch() {
    let (sender, receiver) = mpsc::channel();
    sender.send(results("note", 1)).unwrap();
    drop(sender);
    let (latest, disconnected) = drain(&receiver, "note", false);
    assert!(latest.is_some());
    assert!(disconnected);
}

#[test]
fn only_a_folder_picker_keeps_the_listing_of_an_empty_query() {
    for (lists_without_query, kept) in [(false, None), (true, Some(1))] {
        let (sender, receiver) = mpsc::channel();
        sender.send(results("", 1)).unwrap();
        let (latest, _) = drain(&receiver, "", lists_without_query);
        assert_eq!(latest.map(|batch| batch.candidates), kept);
    }
}

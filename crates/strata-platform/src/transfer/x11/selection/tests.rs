use x11rb::protocol::xproto::SELECTION_REQUEST_EVENT;

use super::super::super::data::SourceSpec;
use super::*;

fn owned_at(time: u32) -> Owned {
    Owned {
        id: SourceId::next(),
        spec: SourceSpec::new(),
        time,
    }
}

fn request_at(time: u32) -> SelectionRequestEvent {
    SelectionRequestEvent {
        response_type: SELECTION_REQUEST_EVENT,
        sequence: 0,
        time,
        owner: 1,
        requestor: 2,
        selection: 3,
        target: 4,
        property: 5,
    }
}

/// The request-time check `request` makes, at the wrap of the server's clock too.
#[test]
fn a_request_made_before_the_selection_was_taken_is_refused() {
    assert!(predates(&request_at(5), &owned_at(10)));
    assert!(!predates(&request_at(10), &owned_at(5)));
    assert!(!predates(&request_at(7), &owned_at(7)));
    assert!(
        !predates(&request_at(CURRENT_TIME), &owned_at(7)),
        "CurrentTime is never early"
    );
    // Taken just before the clock wrapped, asked just after: later, so served.
    assert!(!predates(&request_at(3), &owned_at(u32::MAX - 3)));
    // Asked just before the wrap, taken just after: earlier, so refused.
    assert!(predates(&request_at(u32::MAX - 3), &owned_at(3)));
}

fn take(id: SourceId) -> Owning {
    Owning {
        sel: 1,
        id,
        spec: SourceSpec::new(),
        cancelled: false,
    }
}

#[test]
fn a_cancelled_take_keeps_its_place_so_each_timestamp_goes_to_its_own_take() {
    let (a, b) = (SourceId::next(), SourceId::next());
    let mut takes = Takes::default();
    takes.push(take(a));
    takes.push(take(b));
    assert!(takes.cancel(a));
    assert!(
        takes.next().is_none(),
        "the first timestamp is the cancelled take's"
    );
    assert_eq!(
        takes.next().map(|o| o.id),
        Some(b),
        "the second is the second take's"
    );
    assert!(takes.next().is_none());
    assert!(!takes.cancel(a), "cancelled once");
}

// SPDX-License-Identifier: MIT

use super::*;

mod departing_source;
mod departures;
mod filter_index;
mod lifecycle;
mod local_departures;
mod notifications;

use departing_source::*;

type Events = Rc<RefCell<Vec<BrowserEvent>>>;

fn recorded_events(browser: &Rc<Browser>) -> Events {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    events
}

fn open_path(browser: &Browser) -> Vec<Location> {
    (0..)
        .map_while(|depth| browser.location_at(depth))
        .collect()
}

fn count_events(
    events: &RefCell<Vec<BrowserEvent>>,
    matches: impl Fn(&BrowserEvent) -> bool,
) -> usize {
    events
        .borrow()
        .iter()
        .filter(|event| matches(event))
        .count()
}

fn any_event(events: &RefCell<Vec<BrowserEvent>>, matches: impl Fn(&BrowserEvent) -> bool) -> bool {
    events.borrow().iter().any(matches)
}

/// Runs every row and fails once with every row that went wrong.
fn assert_rows<Row>(rows: impl IntoIterator<Item = Row>, check: impl FnMut(Row) -> Option<String>) {
    let failures: Vec<String> = rows.into_iter().filter_map(check).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

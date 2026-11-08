// SPDX-License-Identifier: MIT

mod catalog;
mod custom;
mod density;
mod live;
mod omarchy;
mod syntax;
mod system_appearance;
mod text_size;

use std::rc::Rc;

use super::{ThemeManager, ThemeTokens};
use gtk::prelude::*;

fn save_custom_theme(manager: &Rc<ThemeManager>, tokens: ThemeTokens) -> std::io::Result<String> {
    let saved = Rc::new(std::cell::RefCell::new(None));
    let done = saved.clone();
    manager.save_custom_theme(tokens, move |result| {
        done.replace(Some(result));
    });
    crate::storage::writer::settle();
    saved.take().expect("theme save finished")
}

fn wait_for_live_theme(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "live theme refresh timed out"
        );
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn has_color(label: &gtk::Label, expected: &gtk::gdk::RGBA) -> bool {
    let actual = label.color();
    (actual.red() - expected.red()).abs() < 0.01
        && (actual.green() - expected.green()).abs() < 0.01
        && (actual.blue() - expected.blue()).abs() < 0.01
}

/// Runs the main loop past the refresh debounce so a missing change is a real absence.
fn settle_live_theme() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
    while std::time::Instant::now() < deadline {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn scheme_color_values(xml: &str) -> Vec<&str> {
    xml.lines()
        .filter_map(|line| {
            let start = line.find("value=\"")? + 7;
            let rest = line.get(start..)?;
            let end = rest.find('"')?;
            Some(&rest[..end])
        })
        .collect()
}

// SPDX-License-Identifier: MIT

use super::footer_prompt::{
    ALL_REPORTS, commit_filter, commit_search, cursor_to_hit, enable_tenxer, highlighted_names,
    hit_cursor, seed_filter_tree, selected_result_names, type_and_submit, wait_results,
};
use super::*;

/// What a single **Esc** may end. While search results show, `fill` is the
/// hits a file verb would take (the fill, else the cursor hit); otherwise it is
/// the directory fill.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Held {
    chord: bool,
    prompt: bool,
    peek: bool,
    filter: Option<String>,
    search: bool,
    highlights: bool,
    visual: bool,
    preview: bool,
    fill: Vec<String>,
    child_column: bool,
}

fn held(fixture: &KeyboardFixture) -> Held {
    let browser = fixture.view.browser();
    let search = fixture.view.listing_search_active();
    Held {
        chord: fixture.shortcuts.armed_chord().is_some(),
        prompt: fixture.shortcuts.open_prompt_kind().is_some(),
        peek: fixture.view.widget().has_css_class("peek-open"),
        filter: fixture.view.listing_filter(),
        search,
        highlights: !highlighted_names(&fixture.view.widget()).is_empty(),
        visual: fixture.shortcuts.visual_text().is_some(),
        preview: fixture.preview.is_enabled(),
        fill: if search {
            selected_result_names(fixture)
        } else {
            fill_names(&browser)
        },
        child_column: browser.location_at(1).is_some(),
    }
}

fn wait_held(fixture: &KeyboardFixture, expected: &Held, context: &str) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while held(fixture) != *expected {
        assert!(
            Instant::now() < deadline,
            "{context}: held {:?}, expected {expected:?}",
            held(fixture)
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    // A later settle must not take a second step for the same press.
    pump(60);
    assert_eq!(held(fixture), *expected, "{context}: settled");
}

fn plain(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(ToString::to_string).collect()
}

fn filtered(query: &str) -> Option<String> {
    Some(query.to_string())
}

fn clear_fill(fixture: &KeyboardFixture) {
    fixture.view.browser().clear_active_selection();
    focus_files(fixture);
}

/// Fills `name` with Space, leaving the cursor on the next item.
fn fill_one(fixture: &KeyboardFixture, name: &str) {
    select_named(fixture, name);
    clear_fill(fixture);
    plain(fixture, Key::space);
    assert_eq!(fill_names(&fixture.view.browser()), [name]);
}

fn open_preview(fixture: &KeyboardFixture) {
    plain(fixture, Key::i);
    wait_until(|| fixture.preview.is_enabled());
}

fn open_peek(fixture: &KeyboardFixture) {
    plain(fixture, Key::i);
    wait_until(|| fixture.view.widget().has_css_class("peek-open"));
}

/// Directory fill b.txt, highlights on "report", filter "alpha" leaving
/// alpha-report.txt, and its preview. Find moves the directory cursor, so it
/// runs before the filter.
fn listing_with_filter(fixture: &KeyboardFixture) -> Held {
    fill_one(fixture, "b.txt");
    type_and_submit(fixture, Key::slash, "report");
    commit_filter(fixture, "alpha");
    wait_until(|| hit_cursor(fixture).as_deref() == Some("alpha-report.txt"));
    open_preview(fixture);
    Held {
        filter: filtered("alpha"),
        highlights: true,
        preview: true,
        fill: names(&["b.txt"]),
        ..Held::default()
    }
}

struct Case {
    name: &'static str,
    modes: &'static [BrowserMode],
    /// Sets the states up and returns what they hold before the first press.
    setup: Box<dyn Fn(&KeyboardFixture) -> Held>,
    /// What survives each press, in order; one more press then changes nothing.
    steps: Vec<Held>,
}

const ALL_MODES: &[BrowserMode] = &[BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];
const PEEK_MODES: &[BrowserMode] = &[BrowserMode::List, BrowserMode::Icons];

fn cases() -> Vec<Case> {
    let listing_tail = || {
        vec![
            Held {
                highlights: true,
                preview: true,
                fill: names(&["b.txt"]),
                ..Held::default()
            },
            Held {
                preview: true,
                fill: names(&["b.txt"]),
                ..Held::default()
            },
            Held {
                fill: names(&["b.txt"]),
                ..Held::default()
            },
            Held::default(),
        ]
    };
    let mut after_filter = vec![Held {
        filter: filtered("alpha"),
        highlights: true,
        preview: true,
        fill: names(&["b.txt"]),
        ..Held::default()
    }];
    after_filter.extend(listing_tail());
    vec![
        Case {
            name: "listing: filter, highlights, preview, then fill",
            modes: ALL_MODES,
            setup: Box::new(listing_with_filter),
            steps: listing_tail(),
        },
        Case {
            name: "listing: visual keeps its fill",
            modes: ALL_MODES,
            setup: Box::new(|fixture| {
                type_and_submit(fixture, Key::slash, "c.txt");
                wait_until(|| focused_name(&fixture.view.browser()) == "c.txt");
                open_preview(fixture);
                plain(fixture, Key::v);
                Held {
                    highlights: true,
                    visual: true,
                    preview: true,
                    fill: names(&["c.txt"]),
                    ..Held::default()
                }
            }),
            steps: vec![
                Held {
                    visual: true,
                    preview: true,
                    fill: names(&["c.txt"]),
                    ..Held::default()
                },
                Held {
                    preview: true,
                    fill: names(&["c.txt"]),
                    ..Held::default()
                },
                Held {
                    fill: names(&["c.txt"]),
                    ..Held::default()
                },
                Held::default(),
            ],
        },
        Case {
            name: "search: visual, preview, then hits restoring the filter",
            modes: ALL_MODES,
            setup: Box::new(|fixture| {
                commit_filter(fixture, "gamma");
                commit_search(fixture, "report");
                wait_results(fixture, &ALL_REPORTS);
                cursor_to_hit(fixture, "alpha-report.txt");
                open_preview(fixture);
                plain(fixture, Key::v);
                Held {
                    filter: filtered("gamma"),
                    search: true,
                    visual: true,
                    preview: true,
                    fill: names(&["alpha-report.txt"]),
                    ..Held::default()
                }
            }),
            steps: vec![
                Held {
                    filter: filtered("gamma"),
                    search: true,
                    preview: true,
                    fill: names(&["alpha-report.txt"]),
                    ..Held::default()
                },
                Held {
                    filter: filtered("gamma"),
                    search: true,
                    fill: names(&["alpha-report.txt"]),
                    ..Held::default()
                },
                Held {
                    filter: filtered("gamma"),
                    ..Held::default()
                },
                Held::default(),
            ],
        },
        Case {
            name: "peek closes before the filter",
            modes: PEEK_MODES,
            setup: Box::new(|fixture| {
                type_and_submit(fixture, Key::slash, "report");
                commit_filter(fixture, "reports");
                wait_until(|| hit_cursor(fixture).as_deref() == Some("reports"));
                open_peek(fixture);
                Held {
                    peek: true,
                    filter: filtered("reports"),
                    highlights: true,
                    ..Held::default()
                }
            }),
            steps: vec![
                Held {
                    filter: filtered("reports"),
                    highlights: true,
                    ..Held::default()
                },
                Held {
                    highlights: true,
                    ..Held::default()
                },
                Held::default(),
            ],
        },
        Case {
            name: "peek closes before search hits",
            modes: PEEK_MODES,
            setup: Box::new(|fixture| {
                commit_search(fixture, "report");
                wait_results(fixture, &ALL_REPORTS);
                cursor_to_hit(fixture, "reports");
                open_peek(fixture);
                Held {
                    peek: true,
                    search: true,
                    fill: names(&["reports"]),
                    ..Held::default()
                }
            }),
            steps: vec![
                Held {
                    search: true,
                    fill: names(&["reports"]),
                    ..Held::default()
                },
                Held::default(),
            ],
        },
        Case {
            name: "an armed chord cancels before the listing",
            modes: ALL_MODES,
            setup: Box::new(|fixture| {
                let before = listing_with_filter(fixture);
                plain(fixture, Key::g);
                Held {
                    chord: true,
                    ..before
                }
            }),
            steps: after_filter.clone(),
        },
        Case {
            name: "a go prompt cancels alone",
            modes: ALL_MODES,
            setup: Box::new(|fixture| {
                let before = listing_with_filter(fixture);
                plain(fixture, Key::g);
                plain(fixture, Key::space);
                fixture.shortcuts.prompt().set_text("..");
                Held {
                    prompt: true,
                    ..before
                }
            }),
            steps: after_filter,
        },
        Case {
            name: "a Miller column outlives Escape",
            modes: &[BrowserMode::Columns],
            setup: Box::new(|fixture| {
                move_to_named(fixture, &fixture.view.browser(), "reports");
                plain(fixture, Key::i);
                wait_until(|| location_ends_with(fixture.view.browser().location_at(1), "reports"));
                plain(fixture, Key::space);
                Held {
                    fill: names(&["reports"]),
                    child_column: true,
                    ..Held::default()
                }
            }),
            steps: vec![Held {
                child_column: true,
                ..Held::default()
            }],
        },
    ]
}

#[test]
fn tenxer_escape_ends_one_interaction_per_press() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::escape_precedence::tenxer_escape_ends_one_interaction_per_press",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            let preferences = enable_tenxer(&fixture);
            preferences.set_filter_include_subfolders(false);
            let browser = fixture.view.browser();
            let folder = fixture._directory.path().to_path_buf();

            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                wait_loaded(&browser, 0);
                for case in cases()
                    .into_iter()
                    .filter(|case| case.modes.contains(&mode))
                {
                    let context = format!("{mode:?} {}", case.name);
                    clear_fill(&fixture);
                    wait_held(
                        &fixture,
                        &Held::default(),
                        &format!("{context}: clean start"),
                    );
                    let before = (case.setup)(&fixture);
                    wait_held(&fixture, &before, &format!("{context}: before Esc"));

                    let steps = case.steps.iter().chain(case.steps.last());
                    for (press, expected) in steps.enumerate() {
                        plain(&fixture, Key::Escape);
                        let context = format!("{context}: Esc {}", press + 1);
                        wait_held(&fixture, expected, &context);
                        assert!(file_panes_have_focus(&fixture), "{context}: focus");
                        assert_eq!(
                            browser
                                .location_at(0)
                                .and_then(|at| at.native_path().map(Into::into)),
                            Some(folder.clone()),
                            "{context}: location"
                        );
                        assert!(fixture.window.is_visible(), "{context}: window");
                    }
                    if mode == BrowserMode::Columns {
                        browser.close_column(1);
                    }
                }
            }
        },
    );
}

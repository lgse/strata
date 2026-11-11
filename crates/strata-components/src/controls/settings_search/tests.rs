#![expect(
    clippy::single_range_in_vec_init,
    reason = "matched text is a list of ranges: a one-range list is meant, not an array of its bounds"
)]

use super::fixture::{self, KEYBOARD_SHORTCUTS, SHORTCUTS};
use super::matching::{MAX_QUERY_CHARS, Prepared, normalized, title_ranges};
use super::state::Change;
use super::{IndexPage, IndexRow, SearchIndex, SettingsSearch, SettingsSearchState};

#[cfg(not(debug_assertions))]
mod perf;

/// What `query` finds in `index`: the best page, and the shown rows' keys.
fn find(index: &SearchIndex, query: &str) -> (Option<String>, Vec<String>) {
    let m = Prepared::new(index).find(&normalized(query));
    let page = m.best.map(|(.., p)| index.pages[p].key.clone());
    let rows = index
        .rows
        .iter()
        .zip(&m.rows)
        .filter(|(_, shown)| **shown)
        .map(|(r, _)| r.key.clone())
        .collect();
    (page, rows)
}

fn keyboard_rows() -> Vec<String> {
    SHORTCUTS.iter().map(|(k, ..)| k.to_string()).collect()
}

/// `settings/preferences`, Settings search: each query bullet over Strata's own index. Each
/// case is (bullet, queries, the page it opens or `None` for any, rows it shows, rows it
/// hides).
#[test]
fn strata_queries_open_their_page_and_show_their_rows() {
    let index = fixture::index();
    type Case = (
        &'static str,
        &'static [&'static str],
        Option<&'static str>,
        &'static [&'static str],
        &'static [&'static str],
    );
    let cases: &[Case] = &[
        (
            "Queries match row titles and aliases, so \"font size\" finds Text size",
            &["font size", "text size"],
            Some("theme"),
            &["text"],
            &[],
        ),
        (
            "Query words of four or more letters tolerate one typo",
            &["tezt size"],
            Some("theme"),
            &["text"],
            &[],
        ),
        (
            "Searching \"Rename\" or \"F2\" opens Keyboard",
            &["Rename", "F2"],
            Some("keyboard"),
            &["keymap.rename", KEYBOARD_SHORTCUTS],
            &["keymap.undo", "keymap.copy"],
        ),
        (
            "Searching \"Ctrl+R\" lists every row that has both a Ctrl key",
            &["Ctrl+R"],
            Some("keyboard"),
            &["keymap.redo", "keymap.refresh"],
            &["keymap.undo", "keymap.copy", "keymap.rename"],
        ),
        (
            "That holds even when only the 10xer table matches, as for \"Sort chord\"",
            &["Sort chord"],
            Some("keyboard"),
            &["keymap.sort_chord", KEYBOARD_SHORTCUTS],
            &["keymap.copy"],
        ),
        (
            "Searching a custom action's name shows the Keyboard row of the slot that runs it",
            &["Resize images"],
            Some("keyboard"),
            &["keymap.custom_action_1"],
            &["keymap.copy"],
        ),
        (
            "A query whose best match is a setting's title or alias opens that setting's page",
            &["copy"],
            Some("actions"),
            &["import-action", "version", "keymap.copy"],
            &[],
        ),
        (
            "Searching \"license\" opens About with the License row",
            &["license"],
            Some("about"),
            &["license"],
            &[],
        ),
        (
            "searching \"custom actions\" opens Actions",
            &["custom actions"],
            Some("actions"),
            &["custom-actions"],
            &[],
        ),
        (
            "Searching \"render documents\" or \"markdown\" shows General's Render documents",
            &["render documents", "markdown"],
            Some("general"),
            &["render-documents"],
            &[],
        ),
        (
            "A query that does not match Custom actions hides the installed-actions list",
            &["import"],
            Some("actions"),
            &["import-action"],
            &["custom-actions"],
        ),
        (
            "the managed-install row shows only for queries matching Check for updates, so \"channel\" hides it",
            &["channel"],
            Some("updates"),
            &["channel"],
            &["check"],
        ),
        (
            "Searching \"miller\" finds Show sizes in Columns view",
            &["miller"],
            None,
            &["columns-sizes"],
            &[],
        ),
        (
            "Searching \"catppuccin\", \"brand palette\", or \"monochrome\" finds Appearance's File icon colors row",
            &["catppuccin", "brand palette", "monochrome"],
            Some("theme"),
            &["file-icon-colors"],
            &[],
        ),
        (
            "Searching \"3d\", \"mesh\", or \"printing\" shows Appearance's STL model thumbnails row",
            &["3d", "mesh", "printing"],
            Some("theme"),
            &["stl-thumbnails"],
            &[],
        ),
        (
            "Searching \"Corner roundness\", \"radius\", or \"rounded\" shows Appearance's Corner roundness row",
            &["Corner roundness", "radius", "rounded"],
            Some("theme"),
            &["roundness"],
            &[],
        ),
        (
            "Searching \"Show thumbnails\" or \"photos\"",
            &["Show thumbnails", "photos"],
            Some("theme"),
            &["show-thumbnails"],
            &[],
        ),
        (
            "Searching \"font\" shows Appearance's Interface font row",
            &["font"],
            Some("theme"),
            &["interface-font"],
            &[],
        ),
        (
            "Searching \"typeface\" or \"family\" shows the Interface font row",
            &["typeface", "family"],
            Some("theme"),
            &["interface-font"],
            &[],
        ),
        (
            "Searching \"troubleshoot\" or \"ffmpeg\" opens Diagnostics with its Run diagnostics row",
            &["troubleshoot", "ffmpeg"],
            Some("diagnostics"),
            &["diagnostics"],
            &[],
        ),
        (
            "Searching \"add-ons\" or \"plugins\" opens the Integrations page",
            &["add-ons", "plugins"],
            Some("integrations"),
            &["integrations"],
            &[],
        ),
        (
            "Searching \"Magyar\" or \"Hungarian\" shows General's Language row",
            &["Magyar", "Hungarian"],
            Some("general"),
            &["language"],
            &[],
        ),
        (
            "searching \"rollback\", \"downgrade\", or \"previous version\" opens Updates",
            &["rollback", "downgrade", "previous version"],
            Some("updates"),
            &["release"],
            &[],
        ),
        (
            "Typing \"thumbnail workers\" in Search settings reveals the Thumbnail workers control",
            &["thumbnail workers"],
            Some("general"),
            &["thumbnail-workers"],
            &[],
        ),
        (
            "Searching Settings for \"session\", \"reopen\", or \"startup\" finds Restore open tabs",
            &["session", "reopen", "startup"],
            None,
            &["restore-tabs"],
            &[],
        ),
        (
            "Searching \"nfs\", \"smb\", or \"monitor\" shows the Watch network folders row",
            &["nfs", "smb", "monitor"],
            Some("general"),
            &["watch-network-folders"],
            &[],
        ),
        (
            "finds Remember sort and icon size per folder, Default sort, and Forget all folder settings",
            &["per folder"],
            Some("general"),
            &["folder-views", "forget-folder-views"],
            &[],
        ),
        (
            "by words such as \"default sort\"",
            &["default sort"],
            Some("general"),
            &["default-sort"],
            &[],
        ),
        (
            "and \"reset\"",
            &["reset"],
            None,
            &["forget-folder-views"],
            &[],
        ),
        (
            "Searching \"timeout\" finds Search time limit",
            &["timeout"],
            Some("general"),
            &["search-time-limit"],
            &["search-entry-limit"],
        ),
        (
            "and \"memory\" finds Search entry limit",
            &["memory"],
            None,
            &["search-entry-limit"],
            &[],
        ),
        (
            "Searching \"miles\", \"gpx\", or \"elevation\" shows the Units row",
            &["miles", "gpx", "elevation"],
            Some("general"),
            &["units"],
            &[],
        ),
        (
            "Searching \"units\" or \"language\" shows the LANGUAGE & UNITS heading with only the matching row",
            &["units"],
            Some("general"),
            &["units"],
            &["language"],
        ),
    ];
    for (bullet, queries, page, shown, hidden) in cases {
        for q in *queries {
            let (best, rows) = find(&index, q);
            if let Some(page) = page {
                assert_eq!(
                    best.as_deref(),
                    Some(*page),
                    "{bullet}: {q:?} opens its page"
                );
            }
            for r in *shown {
                assert!(
                    rows.iter().any(|x| x == r),
                    "{bullet}: {q:?} shows {r} (shown: {rows:?})"
                );
            }
            for r in *hidden {
                assert!(!rows.iter().any(|x| x == r), "{bullet}: {q:?} hides {r}");
            }
        }
    }
}

/// "Searching \"keybindings\" opens General at Show F1 Shortcuts button and also lists every
/// Keyboard row", and "Searching \"hotkeys\", \"keymap\", \"rebind\", or \"remap\" opens
/// Keyboard with every row listed".
#[test]
fn a_query_for_the_keyboard_page_lists_every_keyboard_row() {
    let index = fixture::index();
    let (page, rows) = find(&index, "keybindings");
    assert_eq!(page.as_deref(), Some("general"), "the setting wins the tie");
    assert!(rows.contains(&"hints".to_string()));
    for q in ["keybindings", "hotkeys", "keymap", "rebind", "remap"] {
        let (page, rows) = find(&index, q);
        for r in keyboard_rows() {
            assert!(rows.contains(&r), "{q:?} lists {r}");
        }
        if q != "keybindings" {
            assert_eq!(page.as_deref(), Some("keyboard"), "{q:?}");
        }
    }
}

/// "In a `STRATA_UPDATER=disabled` build, searching \"release channel\" or \"check for
/// updates\" shows \"No settings match your search.\"; no query finds an Updates row."
#[test]
fn a_page_left_out_of_the_index_is_never_found() {
    let index = fixture::index_without(&["updates"]);
    for q in ["release channel", "check for updates", "rollback"] {
        let (page, rows) = find(&index, q);
        assert_eq!((page, rows), (None, Vec::new()), "{q:?}");
    }
}

/// The left-out rule holds for group members too: a member whose page (or group's page) the
/// index leaves out never shows, so the count and the no-results view agree with the page.
#[test]
fn a_member_on_a_left_out_page_is_never_found() {
    let index = SearchIndex {
        revision: 1,
        pages: vec![IndexPage::new("one", "One")],
        rows: vec![
            IndexRow::new("units", "one", "Units"),
            IndexRow::new("keys", "two", "Shortcuts"),
            IndexRow::member("reload", "two", "keys", "Reload folder"),
        ],
    };
    assert_eq!(find(&index, "reload"), (None, Vec::new()));
    let mut st = SettingsSearchState::new();
    st.set_text("reload");
    let r = st.plan(&index).expect("a query");
    st.store(r);
    assert_eq!(st.count(), Some(0));
    assert!(st.no_results());
}

/// "A query with no match shows \"No settings match your search.\"" and the 128-character
/// limit; an empty query, or one of spaces, filters nothing.
#[test]
fn no_match_a_long_query_and_an_empty_one() {
    let index = fixture::index();
    assert_eq!(find(&index, "zzzz qqqq"), (None, Vec::new()));
    let long = "a ".repeat(MAX_QUERY_CHARS / 2 + 1);
    assert_eq!(
        find(&index, &long),
        (None, Vec::new()),
        "past 128 characters nothing matches"
    );
    let mut st = SettingsSearchState::new();
    for q in ["", "   ", "+-+"] {
        st.set_text(q);
        if let Some(r) = st.plan(&index) {
            st.store(r);
        }
        assert!(!st.searching(), "{q:?} filters nothing");
    }
}

/// "Query words of four or more letters tolerate one typo, six or more two": and none below.
#[test]
fn typos_are_allowed_by_word_length() {
    let index = SearchIndex {
        revision: 1,
        pages: vec![IndexPage::new("p", "P")],
        rows: vec![
            IndexRow::new("tab", "p", "Tab"),
            IndexRow::new("text", "p", "Text"),
            IndexRow::new("folder", "p", "Folders"),
        ],
    };
    let shows = |q: &str, r: &str| find(&index, q).1.contains(&r.to_string());
    assert!(!shows("tbx", "tab"), "three letters allow no typo");
    assert!(shows("tezt", "text"), "four allow one");
    assert!(!shows("tezz", "text"), "but not two");
    assert!(shows("foldre", "folder"), "six allow two");
    assert!(!shows("fxldxx", "folder"), "but not three");
}

fn translated(rows: Vec<IndexRow>) -> SearchIndex {
    SearchIndex {
        revision: 1,
        pages: vec![IndexPage::new("general", "Allgemein")],
        rows,
    }
}

/// "Queries also match a row's translated title and the current language's keywords" (with
/// Strata's "Miniaturansichten" under Deutsch), and the English title still counts.
#[test]
fn the_translated_title_the_english_one_and_the_keywords_all_find_a_row() {
    let index = translated(vec![
        IndexRow::new("show-thumbnails", "general", "Miniaturansichten anzeigen")
            .source_title("Show thumbnails")
            .keywords("Bilder Fotos"),
    ]);
    for q in ["Miniaturansichten", "show thumbnails", "fotos", "allgemein"] {
        assert_eq!(find(&index, q).1, ["show-thumbnails"], "{q:?}");
    }
}

/// The Han and kana bullets: a run of two or more such characters matches inside a word; one
/// doesn't; a query may run across the spaces a title puts around Latin words.
#[test]
fn chinese_and_japanese_queries_match_inside_words() {
    let index = translated(vec![
        IndexRow::new("video", "general", "硬件加速视频预览"),
        IndexRow::new("hints", "general", "顯示 F1 快捷鍵按鈕"),
        IndexRow::new("default-directory", "general", "デフォルトのディレクトリ"),
        IndexRow::new("omarchy-variant", "general", "Omarchy 主题变体"),
        IndexRow::new("omarchy-variant-tw", "general", "Omarchy 變體"),
    ]);
    assert_eq!(find(&index, "视频预览").1, ["video"]);
    assert_eq!(find(&index, "快捷鍵按鈕").1, ["hints"]);
    assert_eq!(
        find(&index, "の").1,
        Vec::<String>::new(),
        "one kana doesn't match inside a word"
    );
    assert_eq!(find(&index, "Omarchy主题变体").1, ["omarchy-variant"]);
    // Strata's typo rule finds the other variant too: "omarchy變體" is two edits from the
    // title word "omarchy".
    assert!(
        find(&index, "Omarchy變體")
            .1
            .contains(&"omarchy-variant-tw".to_string())
    );
}

#[test]
fn matched_text_marks_whole_words_prefixes_and_han_runs_in_the_shown_title() {
    assert_eq!(title_ranges("Text size", "size"), [5..9]);
    assert_eq!(title_ranges("Text size", "te si"), [0..2, 5..7]);
    assert_eq!(
        title_ranges("Text size", "tezt"),
        [],
        "a typo marks nothing"
    );
    assert_eq!(
        title_ranges("Thumbnail size", "size thumbnail"),
        [0..9, 10..14]
    );
    assert_eq!(
        title_ranges("Größe der Schrift", "grö"),
        [0..4],
        "whole characters, by bytes"
    );
    assert_eq!(title_ranges("硬件加速视频预览", "视频预览"), [12..24]);
    assert_eq!(title_ranges("Drag & drop", "drop"), [7..11]);
}

/// The marks follow the matcher's own normalisation (`normalized`): the whole title lowercased
/// (a final Σ is ς) and split after lowercasing (İ lowers to i and a combining dot, which
/// splits), with the shown title's words joined as the matcher joins them. A range never
/// splits a character.
#[test]
fn matched_text_follows_the_matchers_lowercasing() {
    let cases: [(&str, &str, &[std::ops::Range<usize>]); 6] = [
        ("ΟΔΟΣ", "οδος", &[0..8]),
        ("ΟΔΟΣ", "ΟΔΟΣ", &[0..8]),
        ("Οδός ΟΔΟΣ", "οδος", &[9..17]),
        ("İstanbul", "istanbul", &[0..9]),
        ("İstanbul", "istan", &[0..6]),
        ("Thumbnail size", "thumbnails", &[0..9, 10..11]),
    ];
    let mut wrong = Vec::new();
    for (title, query, want) in cases {
        let q = normalized(query);
        let index = translated(vec![IndexRow::new("r", "general", title)]);
        assert_eq!(find(&index, &q).1, ["r"], "{title:?} matches {query:?}");
        let got = title_ranges(title, &q);
        assert!(
            got.iter()
                .all(|r| title.is_char_boundary(r.start) && title.is_char_boundary(r.end)),
            "{title:?}: {got:?} splits a character"
        );
        if got != want {
            wrong.push((title, query, got));
        }
    }
    assert!(wrong.is_empty(), "marked wrongly: {wrong:?}");
}

/// "Typing in Search settings ... opens the best-matching page"; clearing a query with no
/// match goes back to the first page; a new query drops a jump's highlight.
#[test]
fn a_new_query_opens_its_best_page_and_clearing_no_results_goes_to_the_first() {
    let index = fixture::index();
    let mut st = SettingsSearchState::new();
    let type_in = |st: &mut SettingsSearchState, q: &str| {
        st.set_text(q);
        st.plan(&index).map(|r| st.store(r))
    };
    assert_eq!(
        type_in(&mut st, "license"),
        Some(Change::Query(Some("about".into())))
    );
    assert_eq!(st.count(), Some(1));
    assert_eq!(
        type_in(&mut st, "license"),
        None,
        "the same query searches nothing again"
    );
    assert_eq!(
        type_in(&mut st, "License "),
        Some(Change::Query(Some("about".into()))),
        "new text with the same words opens its page again, as Strata's every text change does"
    );
    assert_eq!(
        type_in(&mut st, ""),
        Some(Change::Query(None)),
        "clearing stays on the page"
    );
    assert_eq!(type_in(&mut st, "zzzz"), Some(Change::Query(None)));
    assert!(st.no_results());
    assert_eq!(
        type_in(&mut st, ""),
        Some(Change::Query(Some("general".into())))
    );
    assert!(!st.searching() && !st.no_results());
}

#[test]
fn a_new_index_revision_refilters_without_opening_a_page() {
    let mut index = fixture::index();
    let mut st = SettingsSearchState::new();
    st.set_text("resize");
    let r = st.plan(&index).expect("a new query");
    st.store(r);
    let before = st.count().expect("a query");
    assert!(st.prepared().is_some_and(|p| {
        st.matches()
            .is_some_and(|m| m.rows[p.row_by_key["keymap.custom_action_1"]])
    }));
    index.rows.retain(|r| r.key != "keymap.custom_action_1");
    index.revision += 1;
    let r = st.plan(&index).expect("a new revision");
    assert_eq!(st.store(r), Change::Rows);
    assert_eq!(
        st.count(),
        Some(before - 2),
        "the action's row is gone, and the group it alone showed"
    );
}

/// Strata's title bonus: a query equal to a title outranks one inside a title, which
/// outranks the same words matched separately, whatever the index order.
#[test]
fn a_whole_title_match_ranks_first() {
    let index = SearchIndex {
        revision: 1,
        pages: vec![IndexPage::new("one", "One"), IndexPage::new("two", "Two")],
        rows: vec![
            IndexRow::new("limit", "one", "Size limit"),
            IndexRow::new("size", "two", "Size"),
        ],
    };
    assert_eq!(
        find(&index, "size").0.as_deref(),
        Some("two"),
        "equal to a title"
    );
    let index = SearchIndex {
        revision: 1,
        pages: vec![IndexPage::new("one", "One"), IndexPage::new("two", "Two")],
        rows: vec![
            IndexRow::new("apart", "one", "Limit size"),
            IndexRow::new("inside", "two", "Size limits"),
        ],
    };
    assert_eq!(
        find(&index, "size limit").0.as_deref(),
        Some("two"),
        "inside a title"
    );
}

struct Host {
    search: SettingsSearchState,
    index: SearchIndex,
    went: Vec<(String, Option<String>)>,
}

fn search() -> SettingsSearch<Host> {
    SettingsSearch::new(
        "s",
        |h: &Host| &h.search,
        |h: &mut Host| &mut h.search,
        |h: &Host| &h.index,
    )
    .on_navigate(|h: &mut Host, to, _| h.went.push((to.page, to.row)))
}

/// Strata's `heading_shown`: a heading shows while a row under it does, and always when no
/// indexed row is under it; navigation lists pages with a shown row.
#[test]
fn headings_and_pages_follow_the_rows_under_them() {
    let mut h = Host {
        went: Vec::new(),
        search: SettingsSearchState::new(),
        index: SearchIndex {
            revision: 1,
            pages: vec![IndexPage::new("one", "One"), IndexPage::new("two", "Two")],
            rows: vec![
                IndexRow::new("units", "one", "Units").section("language-units"),
                IndexRow::new("language", "one", "Language").section("language-units"),
                IndexRow::new("glow", "two", "Element glow").section("effects"),
            ],
        },
    };
    let s = search();
    assert!(
        s.shows_section(&h, "effects") && s.shows_page(&h, "two"),
        "nothing filters yet"
    );
    h.search.set_text("units");
    let r = h.search.plan(&h.index).expect("a query");
    h.search.store(r);
    assert!(s.shows_section(&h, "language-units"));
    assert!(!s.shows_section(&h, "effects"), "no row under it shows");
    assert!(
        s.shows_section(&h, "about-links"),
        "no indexed row is under it"
    );
    assert!(s.shows_page(&h, "one") && !s.shows_page(&h, "two"));
    assert!(s.shows_row(&h, "units") && !s.shows_row(&h, "language"));
    assert!(
        s.shows_row(&h, "not-indexed"),
        "a row the index doesn't list always shows"
    );
}

/// Enter reads the index as it is now: one the host changed since the last tick is searched
/// first, so the jump never follows a stale row position.
#[test]
fn enter_searches_a_changed_index_before_it_jumps() {
    let mut tx = rune::Tx::default();
    let mut h = Host {
        went: Vec::new(),
        search: SettingsSearchState::new(),
        index: SearchIndex {
            revision: 1,
            pages: vec![IndexPage::new("one", "One"), IndexPage::new("two", "Two")],
            rows: vec![IndexRow::new("units", "one", "Units")],
        },
    };
    let s = search();
    h.search.set_text("units");
    s.refresh(&mut h, &mut tx);
    h.went.clear();
    h.index
        .rows
        .insert(0, IndexRow::new("glow", "two", "Element glow"));
    h.index.revision += 1;
    assert!(s.jump(&mut h, &mut tx));
    assert_eq!(h.search.highlighted(), Some("units"));
    assert_eq!(h.went, [("one".to_owned(), Some("units".to_owned()))]);
}

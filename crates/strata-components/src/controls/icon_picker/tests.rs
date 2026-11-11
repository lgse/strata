mod perf;

use std::borrow::Cow;
use std::rc::Rc;

use super::matching::{Hit, Query, fold, keyword_range, name_ranges, words};
use super::*;

fn kw(list: &[&'static str]) -> Vec<Cow<'static, str>> {
    list.iter().map(|&w| Cow::Borrowed(w)).collect()
}

fn hit(query: &str, name: &str, keywords: &[&'static str]) -> Option<Hit> {
    Query::new(query).hit(&words(name, &kw(keywords)))
}

#[test]
fn each_query_word_must_start_a_word_of_the_name() {
    assert!(matches!(
        hit("grin", "grinning face", &[]),
        Some(Hit::Name(_))
    ));
    assert!(matches!(
        hit("face grin", "grinning face", &[]),
        Some(Hit::Name(_))
    ));
    assert!(
        hit("rinning", "grinning face", &[]).is_none(),
        "a word's middle isn't its start"
    );
    assert!(
        hit("grin cat", "grinning face", &[]).is_none(),
        "every word must match"
    );
}

#[test]
fn a_keyword_matches_when_the_name_does_not() {
    assert_eq!(
        hit("happy", "grinning face", &["cheerful", "happy", "smile"]),
        Some(Hit::Keyword(1, 5))
    );
    // Every query word must start some keyword; GTK matches each keyword whole.
    assert!(hit("happy cheer", "grinning face", &["cheerful", "happy"]).is_some());
    assert!(hit("happy dog", "grinning face", &["cheerful", "happy"]).is_none());
    // Words split between the name and the keywords match neither, as in GTK.
    assert!(hit("grin happy", "grinning face", &["happy"]).is_none());
}

#[test]
fn case_and_latin_accents_fold_on_both_sides() {
    assert!(hit("CAFE", "Café", &[]).is_some());
    assert!(hit("cote", "flag: Côte d’Ivoire", &[]).is_some());
    assert!(hit("piñ", "pinata", &[]).is_some());
    assert_eq!(fold("ÀÉÎõü"), "aeiou");
}

#[test]
fn a_blank_query_matches_nothing() {
    assert!(Query::new("").is_empty());
    assert!(Query::new(" :- ").is_empty());
    assert!(hit("", "grinning face", &[]).is_none());
}

#[test]
fn a_name_hit_lights_each_query_words_share_of_its_name_word() {
    let w = words("flag: Côte d’Ivoire", &[]);
    let h = Query::new("cot iv").hit(&w).expect("a hit");
    let name = "flag: Côte d’Ivoire";
    let lit: Vec<&str> = name_ranges(name, &w, &h)
        .into_iter()
        .map(|r| &name[r])
        .collect();
    assert_eq!(lit, ["Côt", "Iv"]);
    // A keyword hit lights nothing in the name.
    let w = words("grinning face", &kw(&["happy"]));
    let h = Query::new("hap").hit(&w).expect("a hit");
    assert!(name_ranges("grinning face", &w, &h).is_empty());
    // It lights the typed share of the keyword instead.
    let Hit::Keyword(0, chars) = h else {
        panic!("{h:?}")
    };
    assert_eq!(&"happy"[keyword_range("happy", chars)], "hap");
    assert_eq!(&"Côte"[keyword_range("Côte", 3)], "Côt");
}

#[test]
fn remember_puts_a_choice_first_once_and_keeps_twenty_one() {
    let mut recent = Vec::new();
    for i in 0..30 {
        remember(&mut recent, Choice::emoji(format!("e{i}")));
    }
    assert_eq!(recent.len(), RECENT_MAX);
    assert_eq!(recent[0], Choice::emoji("e29"));
    remember(&mut recent, Choice::emoji("e20"));
    assert_eq!(recent[0], Choice::emoji("e20"));
    assert_eq!(
        recent
            .iter()
            .filter(|c| **c == Choice::emoji("e20"))
            .count(),
        1
    );
    assert_eq!(recent.len(), RECENT_MAX);
}

fn two_categories() -> Rc<PickerSource> {
    let cats = vec![
        PickerCategory::new("a", "A", Choice::emoji("🅰\u{fe0f}")),
        PickerCategory::new("b", "B", Choice::emoji("🅱\u{fe0f}")),
    ];
    let items = vec![
        PickerItem::new(Choice::emoji("🚀"), "rocket", 1).keywords(["space"]),
        PickerItem::new(Choice::emoji("😀"), "grinning face", 0).keywords(["happy"]),
        PickerItem::new(Choice::emoji("😀"), "a second grinning face", 0),
        PickerItem::new(Choice::emoji("🪐"), "ringed planet", 1).keywords(["space"]),
        PickerItem::new(Choice::emoji("x"), "out of range", 7),
    ];
    Rc::new(PickerSource::new(cats, items))
}

#[test]
fn a_source_drops_repeated_choices_and_unknown_categories() {
    let s = two_categories();
    assert_eq!(s.items().len(), 3);
    assert_eq!(s.find(&Choice::emoji("😀")), Some(1));
    assert_eq!(s.find(&Choice::emoji("x")), None);
}

#[test]
fn the_catalog_names_icons_by_their_stems_in_two_categories() {
    let s = PickerSource::catalog(&strata_kit::icons::lucide::SET);
    let named = |n: &str| s.find(&Choice::icon(n.to_owned())).map(|i| &s.items()[i]);
    let open = named("strata-folder-open").expect("in the catalog");
    assert_eq!((open.name.as_ref(), open.category), ("folder open", 0));
    let rust = named("strata-ctp-rust").expect("in the catalog");
    assert_eq!((rust.name.as_ref(), rust.category), ("rust", 1));
    assert_eq!(s.items().len(), strata_kit::icons::lucide::SET.all.len());
}

#[test]
fn a_search_spans_every_category_in_order_and_clearing_returns_to_the_tab() {
    let mut st = IconPickerState::new(two_categories());
    st.select_page(Page::Category(1));
    st.set_query("space");
    assert_eq!(st.page(), Page::Results);
    assert_eq!(st.results(), Some(2));
    assert_eq!(
        st.cursor_choice(),
        Some(&Choice::emoji("🚀")),
        "the first result"
    );
    st.set_query("happy");
    assert_eq!(st.results(), Some(1));
    st.set_query("nothing at all");
    assert_eq!(st.results(), Some(0));
    assert_eq!(st.cursor_choice(), None);
    st.set_query("");
    assert_eq!(st.page(), Page::Category(1));
    assert_eq!(st.results(), None);
}

#[test]
fn a_recent_tab_comes_first_once_there_are_recents_and_opens_first() {
    let mut st = IconPickerState::new(two_categories());
    assert_eq!(st.tabs(), [Page::Category(0), Page::Category(1)]);
    st.set_recent(&[
        Choice::emoji("🪐"),
        Choice::emoji("not offered"),
        Choice::emoji("🚀"),
    ]);
    assert_eq!(st.tabs()[0], Page::Recent);
    assert_eq!(
        st.recent_choices().count(),
        2,
        "the unknown one is left out"
    );
    st.open();
    assert_eq!(st.page(), Page::Recent);
    assert_eq!(st.cursor_choice(), Some(&Choice::emoji("🪐")));
    // A search never lists a recent choice twice: it searches the source alone.
    st.set_query("space");
    assert_eq!(st.results(), Some(2));
    st.set_recent(&[]);
    st.set_query("");
    assert_eq!(
        st.page(),
        Page::Category(0),
        "clearing the query never lands on a Recent page with no tab"
    );
    st.open();
    assert_eq!(st.page(), Page::Category(0));
    // Emptying the recents while their tab shows moves to the first category, grid and all.
    st.set_recent(&[Choice::emoji("🪐")]);
    st.open();
    st.set_recent(&[]);
    assert_eq!(st.page(), Page::Category(0));
    assert_eq!(st.cursor_choice(), Some(&Choice::emoji("😀")));
}

#[test]
fn cell_ids_stay_in_base_plus_the_source_length() {
    use crate::contract::EntrySource;
    let mut st = IconPickerState::new(two_categories()).id_base(100);
    let len = st.source().items().len() as u64;
    let mut ids = Vec::new();
    for page in st.tabs() {
        st.select_page(page);
        ids.extend((0..st.shown.len()).map(|i| st.shown.id_at(i).get()));
    }
    ids.sort_unstable();
    assert_eq!(ids, (100..100 + len).collect::<Vec<_>>());
    let id = crate::contract::EntryId::new;
    assert!(st.shown.item_of(id(99)).is_none());
    assert!(st.shown.item_of(id(100 + len)).is_none());
}

#[test]
fn the_cursor_starts_on_the_current_choice_where_the_page_shows_it() {
    let mut st = IconPickerState::new(two_categories());
    st.set_current(Some(&Choice::emoji("🪐")));
    assert_eq!(
        st.cursor_choice(),
        Some(&Choice::emoji("😀")),
        "not on this page"
    );
    st.select_page(Page::Category(1));
    assert_eq!(st.cursor_choice(), Some(&Choice::emoji("🪐")));
    st.set_query("space");
    assert_eq!(
        st.cursor_choice(),
        Some(&Choice::emoji("🚀")),
        "results start at the first"
    );
}

#[test]
fn an_emoji_draws_with_one_emoji_presentation_selector() {
    let drawn = |c: Choice| drawn_text(&c).into_owned();
    assert_eq!(
        drawn(Choice::emoji("♈")),
        "♈\u{fe0f}",
        "added where missing"
    );
    assert_eq!(
        drawn(Choice::emoji("❤\u{fe0f}")),
        "❤\u{fe0f}",
        "never twice"
    );
    // A keycap holds its selector before U+20E3, a ZWJ sequence in the middle.
    assert_eq!(
        drawn(Choice::emoji("#\u{fe0f}\u{20e3}")),
        "#\u{fe0f}\u{20e3}"
    );
    let rainbow = "🏳\u{fe0f}\u{200d}🌈";
    assert_eq!(drawn(Choice::emoji(rainbow)), rainbow);
    assert_eq!(
        drawn(Choice::icon("strata-pin")),
        "strata-pin",
        "icons as they are"
    );
}

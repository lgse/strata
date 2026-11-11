//! Key routing: the open page's `on_key` hears a key first, and only a key it leaves falls back
//! to the gallery's Escape-to-index. A work package tests its own page's keys the same way:
//! mount its scene with `strata_gallery::runtime`, send keys with `Runtime::key` and read the
//! state, or write a script.

use rune::{Env, Key, KeyEvent, Mods, Runtime, Tx};
use strata_gallery::{Gallery, KeyHook, route_key};

/// A page hook that takes Escape and marks that it did by flipping the theme.
fn takes_escape(g: &mut Gallery, key: &KeyEvent, _tx: &mut Tx) -> bool {
    if key.key == Key::Escape {
        g.dark = !g.dark;
        return true;
    }
    false
}

fn leaves_every_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

fn routed_taking(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    route_key(takes_escape, g, key, tx)
}

fn routed_leaving(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    route_key(leaves_every_key, g, key, tx)
}

/// Page w1 open, with `on_key` routing through `hook` as the app routes through a page's. W1's
/// page is static: nothing on it takes focus or a key, so every key reaches the hook.
fn page_open(hook: KeyHook) -> Runtime<Gallery> {
    let mut g = Gallery::default();
    g.open("w1", false);
    let mut rt = strata_gallery::runtime(g, Env::new(false, 1.0));
    rt.on_key = Some(hook);
    rt.frame(0.0);
    rt
}

fn escape(rt: &mut Runtime<Gallery>) {
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
}

#[test]
fn a_page_that_takes_escape_keeps_its_page_open() {
    let mut rt = page_open(routed_taking);
    escape(&mut rt);
    assert_eq!(rt.state.page, Some("w1"));
    assert!(rt.state.dark, "the page's hook should have run");
}

#[test]
fn escape_that_a_page_leaves_goes_back_to_the_index() {
    let mut rt = page_open(routed_leaving);
    escape(&mut rt);
    assert_eq!(rt.state.page, None);
    assert!(!rt.state.dark);
}

/// The mount the goldens use wires the app's own hooks: page w1's `on_key` leaves Escape, so the
/// gallery goes back to the index.
#[test]
fn the_mounted_gallery_routes_keys_through_the_open_page() {
    let mut g = Gallery::default();
    g.open("w1", false);
    let mut rt = strata_gallery::runtime(g, Env::new(false, 1.0));
    rt.frame(0.0);
    escape(&mut rt);
    assert_eq!(rt.state.page, None);
}

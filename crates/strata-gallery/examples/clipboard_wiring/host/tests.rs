//! The demo host, headless: the clipboard keys reach it past the list, and where a paste
//! goes. (What crosses processes is in `tests/x3.rs`, under W3.)

use std::path::PathBuf;

use rune::{Env, Key, KeyEvent, Mods, Runtime};
use strata_components::collection::element::row_key;

use super::*;
use crate::adapter::FileClipboard;
use crate::take_logged;

/// A fixture folder under the gallery's target directory (never the desktop's /tmp).
fn fixture(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/x3-host")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    for d in ["Documents/Reports", "Music"] {
        std::fs::create_dir_all(root.join(d)).expect("fixture folder");
    }
    for f in ["notes.txt", "todo.txt"] {
        std::fs::write(root.join(f), "x").expect("fixture file");
    }
    root.canonicalize().expect("the root")
}

fn mount(name: &str) -> Runtime<Win> {
    let clip = FileClipboard::new(|w: &mut Win| &mut w.clip);
    let w = Win::new(fixture(name), true, clip);
    let mut rt = Runtime::new(w, view, (900.0, 600.0), Env::new(false, 1.0));
    rt.on_key = Some(on_key);
    rt.poll = Some(poll);
    rt.frame(0.0);
    rt
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        },
    }
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

/// Clicks the row of `name`, in the folder shown.
fn click(rt: &mut Runtime<Win>, name: &str) {
    let id = rt.state.ids.of(&rt.state.folder.join(name));
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(row_key(id).as_str()))
        .unwrap_or_else(|| panic!("the {name} row"))
        .rect;
    let at = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    rt.frame(0.1);
}

fn press(rt: &mut Runtime<Win>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.1);
}

fn alt() -> Mods {
    Mods {
        alt: true,
        ..Mods::default()
    }
}

#[test]
fn the_clipboard_keys_reach_the_host_past_the_focused_list() {
    let mut rt = mount("keys");
    click(&mut rt, "notes.txt");
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.focused && e.path.contains(SCOPE)),
        "the click focuses the list"
    );
    assert_eq!(rt.state.selected(), [rt.state.root.join("notes.txt")]);
    take_logged();
    // With no compositor P2 isn't attached: each key reached the host, which asked P2.
    for (c, ev) in [
        ('c', "copy-error"),
        ('x', "copy-error"),
        ('v', "paste-error"),
    ] {
        rt.key(ctrl(c));
        rt.frame(0.1);
        let logged = take_logged();
        assert!(
            logged
                .iter()
                .any(|l| l["ev"] == ev && l["error"] == "the clipboard isn't attached"),
            "Ctrl+{c}: {logged:?}"
        );
    }
}

#[test]
fn a_paste_goes_into_the_one_folder_explicitly_selected_else_the_folder_shown() {
    let mut rt = mount("destination");
    let root = rt.state.root.clone();
    let w = &mut rt.state;
    // operations/clipboard: Ctrl+V with exactly one folder explicitly selected pastes into that folder.
    w.select(&owned(&["Documents"]));
    assert_eq!(w.paste_destination(), root.join("Documents"));
    // operations/clipboard: With a file, several items, or nothing selected, Ctrl+V pastes into the destination column.
    for picked in [&["todo.txt"][..], &["Documents", "Music"], &[]] {
        w.select(&owned(picked));
        assert_eq!(w.paste_destination(), root, "{picked:?}");
    }
    // operations/clipboard: After going to a parent folder, Ctrl+V pastes into the parent, not into the folder selected on arrival, restored or first.
    rt.update(|w, tx| {
        w.open(&w.root.join("Documents"), tx);
    });
    press(&mut rt, Key::Up, alt());
    let w = &rt.state;
    assert_eq!(w.folder, root);
    assert_eq!(
        w.selected(),
        [root.join("Documents")],
        "the folder we came from is picked"
    );
    assert_eq!(w.paste_destination(), root);
}

#[test]
fn a_folder_reached_through_a_link_is_shown_resolved_and_a_link_out_of_the_root_is_refused() {
    let mut rt = mount("links");
    let root = rt.state.root.clone();
    let outside = root.with_file_name("links-outside");
    std::fs::create_dir_all(&outside).expect("a folder outside");
    std::os::unix::fs::symlink(root.join("Documents"), root.join("link")).expect("a link");
    std::os::unix::fs::symlink(&outside, root.join("out")).expect("a link out");
    rt.update(|w, tx| w.refresh(tx));
    rt.frame(0.1);
    // Shown under the paths a paste reports moved (fsys resolves a source's folder), so a
    // cut made here leaves the clipboard when it moves.
    click(&mut rt, "link");
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(rt.state.folder, root.join("Documents"));
    rt.state.select(&owned(&["Reports"]));
    assert_eq!(rt.state.selected(), [root.join("Documents/Reports")]);
    assert_eq!(rt.state.paste_destination(), root.join("Documents/Reports"));
    press(&mut rt, Key::Up, alt());
    assert_eq!(rt.state.folder, root);
    rt.state.select(&owned(&["link"]));
    assert_eq!(rt.state.paste_destination(), root.join("Documents"));
    // A link out of the root isn't browsed into.
    take_logged();
    click(&mut rt, "out");
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(rt.state.folder, root);
    let logged = take_logged();
    assert!(
        logged.iter().any(|l| l["ev"] == "open-refused"),
        "{logged:?}"
    );
}

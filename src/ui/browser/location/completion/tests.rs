// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs,
    path::Path,
    rc::Rc,
    time::Instant,
};

use gtk::{glib, prelude::*};
use tempfile::tempdir;

use crate::{adapters::LocalFileSource, app::Browser, model::Location};

use super::{
    CompletionCandidate, PathCompletion, Plan, format_highlighted_markup, longest_common_prefix,
    plan_completions,
};

fn suggest_completions(
    input: &str,
    current_dir: Option<&Path>,
    home: &Path,
    show_hidden: bool,
) -> Vec<CompletionCandidate> {
    match plan_completions(input, current_dir, home, show_hidden) {
        Plan::Ready(candidates) => candidates,
        Plan::List(listing) => glib::MainContext::new().block_on(listing.candidates()),
    }
}

#[test]
fn unmapping_the_entry_dismisses_suggestions_and_cancels_pending_results() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::unmapping_the_entry_dismisses_suggestions_and_cancels_pending_results",
        || {
            use crate::{
                adapters::{FsOp, FsQuirks, Quirks},
                test_support::Gate,
            };

            let fixture = tempdir().expect("fixture");
            fs::create_dir_all(fixture.path().join("slow/Alpha")).expect("slow folder");
            let gate = Gate::default();
            let _scope = FsQuirks::install(
                fixture.path(),
                Quirks {
                    before: vec![(FsOp::ListChunks, "slow", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let browser = Browser::new(Rc::new(LocalFileSource));
            let entry = gtk::Entry::new();
            let completion = PathCompletion::attach(&entry, browser.clone(), || true, || {});
            let window = gtk::Window::builder().child(&entry).build();
            window.present();
            wait_until(|| entry.is_mapped());
            entry.set_text(&format!("{}/", fixture.path().display()));
            wait_until(|| completion.popover.is_visible());

            window.set_visible(false);
            assert!(
                !completion.popover.is_visible(),
                "hiding the window dismisses suggestions"
            );
            window.present();
            wait_until(|| entry.is_mapped());
            assert!(
                !completion.popover.is_visible(),
                "restoring does not reopen stale suggestions"
            );
            completion.refresh(&entry, &browser);
            wait_until(|| completion.popover.is_visible());

            entry.set_text(&format!("{}/slow/", fixture.path().display()));
            wait_until(|| gate.reached());
            window.set_visible(false);
            assert!(
                completion.request.borrow().is_none(),
                "unmapping cancels the pending listing"
            );
            gate.release();
            window.present();
            wait_until(|| entry.is_mapped());
            assert!(!completion.popover.is_visible());
            completion.refresh(&entry, &browser);
            wait_until(|| completion.popover.is_visible());
            assert_eq!(completion.candidates.borrow()[0].display_name, "Alpha/");
            window.destroy();
        },
    );
}

#[test]
fn first_reveal_shows_suggestions_after_switching_from_breadcrumbs() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::first_reveal_shows_suggestions_after_switching_from_breadcrumbs",
        || {
            let fixture = tempdir().expect("fixture");
            fs::create_dir(fixture.path().join("Documents")).expect("completion folder");

            let browser = Browser::new(Rc::new(LocalFileSource));
            browser.navigate(Location::local(fixture.path()));

            let entry = gtk::Entry::builder().hexpand(true).build();
            entry.set_text(&fixture.path().to_string_lossy());
            let entry_control = gtk::Box::new(gtk::Orientation::Vertical, 0);
            entry_control.set_hexpand(true);
            entry_control.append(&entry);
            let breadcrumbs = gtk::Label::new(Some("Fixture"));
            let stack = gtk::Stack::builder()
                .hhomogeneous(false)
                .vhomogeneous(false)
                .hexpand(true)
                .build();
            stack.add_named(&breadcrumbs, Some("breadcrumbs"));
            stack.add_named(&entry_control, Some("entry"));
            stack.set_visible_child_name("breadcrumbs");

            let active_stack = stack.clone();
            let completion_browser = browser.clone();
            let completion = PathCompletion::attach(
                &entry,
                browser,
                move || active_stack.visible_child_name().as_deref() == Some("entry"),
                || {},
            );
            let window = gtk::Window::builder()
                .child(&stack)
                .default_width(720)
                .build();
            window.present();
            wait_until(|| breadcrumbs.is_mapped());
            assert!(!entry.is_mapped());

            stack.set_visible_child_name("entry");
            entry.grab_focus();
            completion.refresh(&entry, &completion_browser);
            wait_until(|| entry.is_mapped() && completion.popover.is_visible());
            assert!(completion.list.row_at_index(0).is_some());
            window.destroy();
        },
    );
}

#[test]
fn activating_a_suggestion_does_not_refresh_the_dismissing_list() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::activating_a_suggestion_does_not_refresh_the_dismissing_list",
        || {
            let fixture = tempdir().expect("fixture");
            fs::create_dir_all(fixture.path().join("Documents/Projects"))
                .expect("nested completion folders");

            let browser = Browser::new(Rc::new(LocalFileSource));
            browser.navigate(Location::local(fixture.path()));
            let entry = gtk::Entry::builder().hexpand(true).build();
            entry.set_text(&fixture.path().to_string_lossy());
            let activated = Rc::new(Cell::new(false));
            let activated_from_row = activated.clone();
            let completion = PathCompletion::attach(
                &entry,
                browser.clone(),
                || true,
                move || {
                    activated_from_row.set(true);
                },
            );
            let window = gtk::Window::builder()
                .child(&entry)
                .default_width(720)
                .build();
            window.present();
            wait_until(|| entry.width() >= 700);
            completion.refresh(&entry, &browser);
            wait_until(|| completion.popover.is_visible());
            let before = completion.candidates.borrow().clone();
            let row = completion.list.row_at_index(0).expect("suggestion row");

            completion.list.emit_by_name::<()>("row-activated", &[&row]);

            assert!(activated.get());
            assert!(!completion.popover.is_visible());
            assert_eq!(*completion.candidates.borrow(), before);
            window.destroy();
        },
    );
}

#[test]
fn down_reopens_suggestions_after_tab_completion() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::down_reopens_suggestions_after_tab_completion",
        || {
            let fixture = tempdir().expect("fixture");
            fs::create_dir_all(fixture.path().join("Documents/Projects"))
                .expect("nested completion folder");
            fs::create_dir(fixture.path().join("Downloads")).expect("empty folder");
            let browser = Browser::new(Rc::new(LocalFileSource));
            browser.navigate(Location::local(fixture.path()));
            let entry = gtk::Entry::new();
            let completion = PathCompletion::attach(&entry, browser, || true, || {});
            let window = gtk::Window::builder().child(&entry).build();
            window.present();
            entry.grab_focus();
            wait_until(|| entry.is_mapped());
            let keys = entry
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("completion keyboard controller");
            let press = |key: gtk::gdk::Key| {
                assert!(keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
                ));
            };

            for (prefix, select_first, completed, suggestion) in [
                ("Doc", false, "Documents/", Some("Projects/")),
                ("Do", true, "Documents/", Some("Projects/")),
                ("D", false, "Do", Some("Documents/")),
                ("Down", false, "Downloads/", None),
            ] {
                entry.set_text(&fixture.path().join(prefix).to_string_lossy());
                wait_until(|| completion.popover.is_visible());
                if select_first {
                    press(gtk::gdk::Key::Down);
                }
                press(gtk::gdk::Key::Tab);
                let completed = fixture.path().join(completed);
                wait_until(|| entry.text().as_str() == completed.to_string_lossy());
                wait_until(|| !completion.popover.is_visible());
                press(gtk::gdk::Key::Down);
                if let Some(suggestion) = suggestion {
                    wait_until(|| completion.popover.is_visible());
                    assert_eq!(completion.candidates.borrow()[0].display_name, suggestion);
                    press(gtk::gdk::Key::Down);
                    assert_eq!(completion.selected_index.get(), Some(0));
                    press(gtk::gdk::Key::Escape);
                    wait_until(|| !completion.popover.is_visible());
                    press(gtk::gdk::Key::Down);
                    wait_until(|| completion.popover.is_visible());
                } else {
                    wait_until(|| completion.request.borrow().is_none());
                    assert!(!completion.popover.is_visible());
                    assert!(completion.candidates.borrow().is_empty());
                }
            }
            window.destroy();
        },
    );
}

#[test]
fn tab_alternates_complete_like_tab() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::tab_alternates_complete_like_tab",
        || {
            use crate::ui::keys::{
                Binding, Keymap, Map,
                bindings::{FOCUS_NEXT, FOCUS_PREVIOUS},
                use_keymap,
            };
            use gtk::gdk::{Key, ModifierType};

            let _keymap = use_keymap(
                Keymap::defaults()
                    .with(
                        Map::Default,
                        FOCUS_NEXT,
                        vec![Binding::plain(Key::Tab), Binding::plain(Key::F7)],
                    )
                    .with(
                        Map::Default,
                        FOCUS_PREVIOUS,
                        vec![Binding::shift(Key::Tab), Binding::ctrl(Key::F7)],
                    ),
            );
            let fixture = tempdir().expect("fixture");
            fs::create_dir(fixture.path().join("Documents")).expect("completion folder");
            let browser = Browser::new(Rc::new(LocalFileSource));
            browser.navigate(Location::local(fixture.path()));
            let entry = gtk::Entry::new();
            let completion = PathCompletion::attach(&entry, browser, || true, || {});
            let window = gtk::Window::builder().child(&entry).build();
            window.present();
            entry.grab_focus();
            wait_until(|| entry.is_mapped());
            let keys = entry
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("completion keyboard controller");

            let completed = fixture.path().join("Documents/");
            for modifiers in [ModifierType::empty(), ModifierType::CONTROL_MASK] {
                entry.set_text(&fixture.path().join("Doc").to_string_lossy());
                wait_until(|| completion.popover.is_visible());
                assert!(
                    keys.emit_by_name::<bool>("key-pressed", &[&Key::F7, &0u32, &modifiers]),
                    "{modifiers:?}"
                );
                wait_until(|| entry.text().as_str() == completed.to_string_lossy());
            }
            window.destroy();
        },
    );
}

#[test]
fn typed_paths_are_listed_off_the_ui_thread_and_only_the_latest_text_is_shown() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::typed_paths_are_listed_off_the_ui_thread_and_only_the_latest_text_is_shown",
        || {
            use crate::{
                adapters::{FsOp, FsQuirks, Quirks},
                test_support::Gate,
            };
            use gtk::gdk::Key;

            let fixture = tempdir().expect("fixture");
            for folder in [
                "slow/Alpha",
                "slow/Beta",
                "fast/Gamma",
                "later/Alpha",
                "list/Delta",
                "list/Echo",
                "submit/Alpha",
                "twice/Alpha/Inner",
                "pair/Projects-a",
                "pair/Projects-b",
            ] {
                fs::create_dir_all(fixture.path().join(folder)).expect("fixture folder");
            }
            let gates: Vec<_> = ["slow", "later", "list", "submit", "twice", "pair"]
                .into_iter()
                .map(|folder| (folder, Gate::default()))
                .collect();
            let gate = |folder: &str| {
                gates
                    .iter()
                    .find_map(|(name, gate)| (*name == folder).then_some(gate))
                    .expect("gated folder")
            };
            let (slow, later, list) = (gate("slow"), gate("later"), gate("list"));
            let _scope = FsQuirks::install(
                fixture.path(),
                Quirks {
                    before: gates
                        .iter()
                        .map(|(folder, gate)| (FsOp::ListChunks, *folder, 0, gate.racer()))
                        .collect(),
                    ..Quirks::default()
                },
            );
            let browser = Browser::new(Rc::new(LocalFileSource));
            let entry = gtk::Entry::new();
            let submitted = Rc::new(RefCell::new(Vec::<String>::new()));
            let (submit, submitted_text) = (submitted.clone(), entry.clone());
            let completion = PathCompletion::attach(
                &entry,
                browser,
                || true,
                move || {
                    submit.borrow_mut().push(submitted_text.text().into());
                },
            );
            let window = gtk::Window::builder().child(&entry).build();
            window.present();
            entry.grab_focus();
            wait_until(|| entry.is_mapped());
            let path = |relative: &str| format!("{}/{relative}", fixture.path().display());
            let shown = || {
                completion.popover.is_visible().then(|| {
                    completion
                        .candidates
                        .borrow()
                        .iter()
                        .map(|candidate| candidate.display_name.clone())
                        .collect::<Vec<_>>()
                })
            };

            entry.set_text(&path("slow/"));
            wait_until(|| slow.reached());
            assert_eq!(
                shown(),
                None,
                "nothing is offered while the folder is listed"
            );
            entry.set_text(&path("fast/"));
            wait_until(|| shown().is_some());
            assert_eq!(shown(), Some(vec!["Gamma/".to_owned()]));
            slow.release();
            let settled = Instant::now() + std::time::Duration::from_millis(200);
            while Instant::now() < settled {
                glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                shown(),
                Some(vec!["Gamma/".to_owned()]),
                "an earlier text's listing is dropped"
            );

            entry.set_text(&path("later/Al"));
            wait_until(|| later.reached());
            let keys = entry
                .observe_controllers()
                .iter::<glib::Object>()
                .filter_map(Result::ok)
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("completion keyboard controller");
            let press = |key: gtk::gdk::Key| {
                keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
                )
            };
            assert!(press(gtk::gdk::Key::Tab));
            assert_eq!(
                entry.text().as_str(),
                path("later/Al"),
                "Tab waits for the listing"
            );
            later.release();
            wait_until(|| entry.text().as_str() == path("later/Alpha/"));

            entry.set_text(&path("lis"));
            wait_until(|| shown() == Some(vec!["list/".to_owned()]));
            let mut end = entry.text().chars().count() as i32;
            entry.insert_text("t/", &mut end);
            wait_until(|| list.reached());
            assert!(press(gtk::gdk::Key::Down));
            assert!(press(gtk::gdk::Key::Return));
            assert!(
                submitted.borrow().is_empty(),
                "Enter after Down waits for the listing"
            );
            list.release();
            wait_until(|| submitted.borrow().len() == 1);
            assert_eq!(submitted.borrow()[0], path("list/Delta/"));

            for (shown_before, typed, keys, folder, result) in [
                (
                    None,
                    "submit/Al",
                    &[Key::Tab, Key::Return][..],
                    "submit",
                    "submit/Alpha/",
                ),
                (
                    None,
                    "twice/Al",
                    &[Key::Tab, Key::Tab],
                    "twice",
                    "twice/Alpha/Inner/",
                ),
                (
                    Some("pai"),
                    "pair/Pro",
                    &[Key::Tab, Key::Down, Key::Return],
                    "pair",
                    "pair/Projects-",
                ),
            ] {
                submitted.borrow_mut().clear();
                match shown_before {
                    Some(before) => {
                        entry.set_text(&path(before));
                        wait_until(|| shown().is_some());
                        let mut end = entry.text().chars().count() as i32;
                        entry.insert_text(&typed[before.len()..], &mut end);
                    }
                    None => entry.set_text(&path(typed)),
                }
                wait_until(|| gate(folder).reached());
                for key in keys {
                    assert!(press(*key), "{typed}: {key:?} waits for the listing");
                }
                assert_eq!(entry.text().as_str(), path(typed));
                gate(folder).release();
                let submits = keys.contains(&Key::Return);
                wait_until(|| {
                    completion.request.borrow().is_none()
                        && entry.text().as_str() == path(result)
                        && submitted.borrow().len() == usize::from(submits)
                });
                if submits {
                    assert_eq!(submitted.borrow()[0], path(result), "{typed}");
                }
            }
            window.destroy();
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "completion UI did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn suggest_completions_for_home_shorthand() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();

    fs::create_dir_all(home.join("Documents")).expect("create Documents");
    fs::create_dir_all(home.join("Downloads")).expect("create Downloads");
    fs::create_dir_all(home.join("Desktop")).expect("create Desktop");
    fs::create_dir_all(home.join(".config")).expect("create .config");
    fs::write(home.join("notes.txt"), b"hello").expect("write file");

    let tilde = suggest_completions("~", None, home, false);
    assert_eq!(
        tilde,
        vec![CompletionCandidate {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            parent_hint: "home".to_owned(),
            match_len: 1,
        }]
    );

    let home_slash = suggest_completions("~/", None, home, false);
    let names: Vec<_> = home_slash.iter().map(|c| c.display_name.as_str()).collect();
    assert!(names.contains(&"Desktop/"));
    assert!(names.contains(&"Documents/"));
    assert!(names.contains(&"Downloads/"));
    assert!(!names.contains(&"notes.txt"));
    assert!(!names.contains(&".config/"));

    let with_hidden = suggest_completions("~/", None, home, true);
    let hidden_names: Vec<_> = with_hidden
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert!(hidden_names.contains(&".config/"));

    let prefix_match = suggest_completions("~/Doc", None, home, false);
    assert_eq!(
        prefix_match,
        vec![CompletionCandidate {
            display_name: "Documents/".to_owned(),
            replacement: "~/Documents/".to_owned(),
            parent_hint: "~".to_owned(),
            match_len: 3,
        }]
    );

    let hidden_prefix = suggest_completions("~/.co", None, home, false);
    assert_eq!(
        hidden_prefix,
        vec![CompletionCandidate {
            display_name: ".config/".to_owned(),
            replacement: "~/.config/".to_owned(),
            parent_hint: "~".to_owned(),
            match_len: 3,
        }]
    );
}

#[test]
fn suggest_completions_for_absolute_paths() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path();

    fs::create_dir_all(root.join("sub/alpha")).expect("create alpha");
    fs::create_dir_all(root.join("sub/beta")).expect("create beta");
    fs::write(root.join("sub/albatross.txt"), b"bird").expect("write albatross");

    let sub_str = root.join("sub").to_string_lossy().into_owned();
    let sub_slash = format!("{sub_str}/");
    let completions = suggest_completions(&sub_slash, None, Path::new("/dummy"), false);
    let names: Vec<_> = completions
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert!(names.contains(&"alpha/"));
    assert!(names.contains(&"beta/"));
    assert!(!names.contains(&"albatross.txt"));

    let prefix_query = format!("{sub_str}/al");
    let al_completions = suggest_completions(&prefix_query, None, Path::new("/dummy"), false);
    assert_eq!(
        al_completions,
        vec![CompletionCandidate {
            display_name: "alpha/".to_owned(),
            replacement: format!("{sub_str}/alpha/"),
            parent_hint: sub_str.clone(),
            match_len: 2,
        }]
    );

    let exact_directory = suggest_completions(&sub_str, None, Path::new("/dummy"), false);
    let exact_names: Vec<_> = exact_directory
        .iter()
        .map(|candidate| candidate.display_name.as_str())
        .collect();
    assert_eq!(exact_names, vec!["alpha/", "beta/"]);
}

#[test]
fn suggest_completions_for_relative_paths() {
    let temp = tempdir().expect("tempdir");
    let current = temp.path();

    fs::create_dir_all(current.join("photos/vacation")).expect("create vacation");
    fs::create_dir_all(current.join("photos/work")).expect("create work");

    let relative_slash = suggest_completions("photos/", Some(current), Path::new("/dummy"), false);
    let names: Vec<_> = relative_slash
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["vacation/", "work/"]);
    assert_eq!(
        relative_slash[0].replacement,
        format!("{}/photos/vacation/", current.to_string_lossy())
    );

    let relative_prefix =
        suggest_completions("photos/va", Some(current), Path::new("/dummy"), false);
    assert_eq!(
        relative_prefix,
        vec![CompletionCandidate {
            display_name: "vacation/".to_owned(),
            replacement: format!("{}/photos/vacation/", current.to_string_lossy()),
            parent_hint: current.join("photos").to_string_lossy().into_owned(),
            match_len: 2,
        }]
    );
}

#[test]
fn suggest_completions_handles_empty_and_nonexistent_gracefully() {
    let temp = tempdir().expect("tempdir");
    let current = temp.path();

    let empty = suggest_completions("", None, Path::new("/dummy"), false);
    assert!(empty.is_empty());

    let nonexistent = suggest_completions(
        "/definitely/not/a/real/path",
        None,
        Path::new("/dummy"),
        false,
    );
    assert!(nonexistent.is_empty());

    let empty_with_current = suggest_completions("", Some(current), Path::new("/dummy"), false);
    assert!(empty_with_current.is_empty());
}

#[test]
fn highlighted_names_escape_filename_markup() {
    assert_eq!(
        format_highlighted_markup("<docs>&", 1),
        "<b>&lt;</b>docs&gt;&amp;"
    );
    assert_eq!(format_highlighted_markup("éclair", 1), "<b>é</b>clair");
}

#[test]
fn longest_common_prefix_computes_shared_start() {
    assert_eq!(longest_common_prefix(&[]), None);
    assert_eq!(
        longest_common_prefix(&["~/Documents/".to_owned()]),
        Some("~/Documents/".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&[
            "~/Documents/notes1.txt".to_owned(),
            "~/Documents/notes2.txt".to_owned(),
        ]),
        Some("~/Documents/notes".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&["/var/log/".to_owned(), "/usr/bin/".to_owned(),]),
        Some("/".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&["abc".to_owned(), "xyz".to_owned(),]),
        None
    );
    assert_eq!(
        longest_common_prefix(&["~/éclair/".to_owned(), "~/étude/".to_owned()]),
        Some("~/é".to_owned())
    );
}

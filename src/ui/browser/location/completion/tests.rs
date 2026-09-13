// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, fs, path::Path, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use tempfile::tempdir;

use crate::{adapters::LocalFileSource, app::Browser, model::Location};

use super::{
    CompletionCandidate, PathCompletion, format_highlighted_markup, longest_common_prefix,
    suggest_completions,
};

#[test]
fn first_reveal_sizes_the_popover_to_the_allocated_entry() {
    crate::test_support::gtk_test(
        "ui::browser::location::completion::tests::first_reveal_sizes_the_popover_to_the_allocated_entry",
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
            wait_until(|| stack.width() >= 700);
            assert_eq!(entry.width(), 0, "hidden entry must begin unallocated");

            stack.set_visible_child_name("entry");
            entry.grab_focus();
            completion.refresh(&entry, &completion_browser);
            wait_until(|| entry.width() >= 700 && completion.popover.is_visible());

            assert!(
                (completion.popover.width() - entry.width()).abs() <= 8,
                "first popover width {} must match entry width {}",
                completion.popover.width(),
                entry.width()
            );
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

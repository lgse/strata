// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    adapters::LocalFileSource,
    test_support::gtk_test,
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
        shortcut_footer::ShortcutFooter,
    },
};
use std::{
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};

fn init_repo(path: &Path) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["init", "--quiet"])
        .output()
        .expect("initialize git repository");
    assert!(output.status.success());
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["symbolic-ref", "HEAD", "refs/heads/feature/current-branch"])
        .output()
        .expect("set git branch");
    assert!(output.status.success());
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Git indicator"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn recycled_status_badge_clears_previous_state() {
    gtk_test(
        "ui::browser::git_badge::tests::recycled_status_badge_clears_previous_state",
        || {
            let badge = create_git_badge();
            render_status_badge(&badge, Some(GitStatus::Modified));
            assert!(badge.is_visible());
            assert_eq!(badge.text().as_str(), "M");
            assert!(badge.has_css_class("git-badge-modified"));

            render_status_badge(&badge, Some(GitStatus::Untracked));
            assert_eq!(badge.text().as_str(), "U");
            assert!(!badge.has_css_class("git-badge-modified"));
            assert!(badge.has_css_class("git-badge-untracked"));

            render_status_badge(&badge, None);
            assert!(!badge.is_visible());
            assert!(badge.text().is_empty());
            assert!(!badge.has_css_class("git-badge-untracked"));
        },
    );
}

#[test]
fn branch_indicator_distinguishes_branch_detached_and_absent_heads() {
    gtk_test(
        "ui::browser::git_badge::tests::branch_indicator_distinguishes_branch_detached_and_absent_heads",
        || {
            let indicator = create_git_branch_indicator();
            let label = branch_label(&indicator).expect("branch label");

            render_git_branch(
                &indicator,
                Some(GitHead::Branch("feature/current-branch".to_owned())),
            );
            assert!(indicator.is_visible());
            assert_eq!(label.text().as_str(), "feature/current-branch");
            assert_eq!(
                indicator.tooltip_text().as_deref(),
                Some("feature/current-branch")
            );
            assert!(!indicator.has_css_class("detached"));

            render_git_branch(&indicator, Some(GitHead::Detached("0123abcd".to_owned())));
            assert_eq!(label.text().as_str(), "Detached · 0123abcd");
            assert_eq!(
                indicator.tooltip_text().as_deref(),
                Some("Detached · 0123abcd")
            );
            assert!(indicator.has_css_class("detached"));

            render_git_branch(&indicator, None);
            assert!(!indicator.is_visible());
            assert!(label.text().is_empty());
            assert!(indicator.tooltip_text().is_none());
            assert!(!indicator.has_css_class("detached"));
        },
    );
}

#[test]
fn saved_preference_applies_before_settings_and_live_across_views_and_rebuilds() {
    gtk_test(
        "ui::browser::git_badge::tests::saved_preference_applies_before_settings_and_live_across_views_and_rebuilds",
        || {
            let repo = tempfile::tempdir().expect("repository fixture");
            init_repo(repo.path());
            std::fs::write(repo.path().join("new-file.txt"), "new").expect("write untracked file");
            let location = Location::local(repo.path());
            let manager = PreferenceManager::shared();
            manager.set_git_status_badges(false);

            let views = [
                BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default()),
                BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default()),
            ];
            for view in &views {
                view.state.set_location(&location);
                assert!(!view.state.git_branch_indicator.is_visible());
            }

            manager.set_git_status_badges(true);
            wait_until(|| {
                views
                    .iter()
                    .all(|view| view.state.git_branch_indicator.is_visible())
            });
            for view in &views {
                let label = branch_label(&view.state.git_branch_indicator).expect("branch label");
                assert_eq!(label.text().as_str(), "feature/current-branch");
            }

            let rebuilt_badge = create_git_badge();
            track_git_badge(
                &rebuilt_badge,
                &Location::local(repo.path().join("new-file.txt")),
                false,
            );
            wait_until(|| rebuilt_badge.is_visible());
            assert_eq!(rebuilt_badge.text().as_str(), "U");

            manager.set_git_status_badges(false);
            assert!(!rebuilt_badge.is_visible());
            for view in &views {
                assert!(!view.state.git_branch_indicator.is_visible());
            }
        },
    );
}

#[test]
fn branch_in_footer_survives_hidden_hints_and_clears_outside_git() {
    gtk_test(
        "ui::browser::git_badge::tests::branch_in_footer_survives_hidden_hints_and_clears_outside_git",
        || {
            let repo = tempfile::tempdir().expect("repository fixture");
            init_repo(repo.path());
            let outside = tempfile::tempdir().expect("non-repository fixture");
            let manager = PreferenceManager::shared();
            manager.set_git_status_badges(true);
            manager.set_show_keybinding_hints(false);

            let view = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            let indicator = view.git_branch_indicator();
            footer.set_git_branch(&indicator);
            footer.bind_preferences(&manager);
            assert!(indicator.is_ancestor(footer.widget()));
            assert!(
                indicator
                    .next_sibling()
                    .is_some_and(|widget| widget.has_css_class("shortcut-footer-count"))
            );
            assert!(!footer.widget().is_visible());

            view.state.set_location(&Location::local(repo.path()));
            wait_until(|| indicator.is_visible());
            assert!(footer.widget().is_visible());
            assert_eq!(
                indicator.tooltip_text().as_deref(),
                Some("feature/current-branch")
            );

            view.state.set_location(&Location::local(outside.path()));
            assert!(!indicator.is_visible());
            assert!(!footer.widget().is_visible());

            manager.set_show_keybinding_hints(true);
            manager.set_git_status_badges(false);
        },
    );
}

#[test]
fn virtual_location_replaces_a_tracked_native_badge() {
    gtk_test(
        "ui::browser::git_badge::tests::virtual_location_replaces_a_tracked_native_badge",
        || {
            let badge = create_git_badge();
            track_git_badge(
                &badge,
                &Location::local("/nonexistent/repository/item"),
                false,
            );
            track_git_badge(&badge, &Location::uri("trash:///item"), false);
            assert!(!badge.is_visible());
            assert!(badge.text().is_empty());
        },
    );
}

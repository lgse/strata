// SPDX-License-Identifier: MIT

use std::{cell::RefCell, sync::mpsc};

use super::super::release_picker::{ReleasePicker, ReleaseSource, release_picker};
use super::super::*;
use super::{FakeInstaller, page_text, wait_until};
use crate::{services::ReleaseChoice, test_support::gtk_test};

type Lists = Rc<RefCell<Vec<(Channel, mpsc::Sender<Result<Vec<ReleaseChoice>, String>>)>>>;

#[derive(Default)]
struct FakeReleases {
    lists: Lists,
    installer: FakeInstaller,
}

impl FakeReleases {
    fn source(&self) -> ReleaseSource {
        let lists = self.lists.clone();
        ReleaseSource {
            list: Rc::new(move |channel| {
                let (sender, receiver) = mpsc::channel();
                lists.borrow_mut().push((channel, sender));
                receiver
            }),
            resolve_commit: Rc::new(|mut release| {
                let (sender, receiver) = mpsc::channel();
                release.commit = Some("0123abcd".to_owned());
                sender.send(release).expect("commit receiver");
                receiver
            }),
            launcher: self.installer.launcher(),
        }
    }

    fn requested(&self) -> Vec<Channel> {
        self.lists
            .borrow()
            .iter()
            .map(|(channel, _)| *channel)
            .collect()
    }

    fn answer(&self, result: Result<Vec<ReleaseChoice>, String>) {
        self.lists
            .borrow()
            .last()
            .expect("a release list request")
            .1
            .send(result)
            .expect("the picker is still listening");
    }
}

fn choice(version: &str, kind: BuildKind) -> ReleaseChoice {
    ReleaseChoice {
        release: ReleaseMetadata {
            version: version.to_owned(),
            url: format!("https://github.com/lgse/strata/releases/tag/v{version}"),
            notes: String::new(),
            note_blocks: Vec::new(),
            kind,
            tag: format!("v{version}"),
            published_at: Some("2026-09-20T12:00:00Z".to_owned()),
            commit: None,
        },
        install: InstallRequest {
            tag: format!("v{version}"),
            asset_name: format!(
                "strata-{version}-{}-unknown-linux-gnu.tar.gz",
                std::env::consts::ARCH
            ),
            advertised_url: format!("http://127.0.0.1:9/{version}.tar.gz"),
        },
    }
}

fn open_picker(releases: &FakeReleases) -> (gtk::Window, ReleasePicker) {
    let picker = release_picker(
        &PreferenceManager::shared(),
        install_guard(),
        releases.source(),
    );
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&BlurBin::new(&picker.row)));
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(900)
        .child(&overlay)
        .build();
    window.present();
    wait_until("the picker to be mapped", || picker.button.is_mapped());
    (window, picker)
}

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        found.extend(descendants(&widget));
    }
    found
}

fn options(picker: &ReleasePicker) -> Vec<gtk::Button> {
    descendants(picker.menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .collect()
}

fn option(picker: &ReleasePicker, version: &str) -> gtk::Button {
    options(picker)
        .into_iter()
        .find(|option| page_text(option.upcast_ref()).starts_with(&format!("v{version}\n")))
        .unwrap_or_else(|| panic!("no option for v{version}"))
}

fn button_labelled(window: &gtk::Window, label: &str) -> Option<gtk::Button> {
    descendants(window.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| button.label().as_deref() == Some(label))
}

#[test]
fn the_menu_lists_its_channel_s_releases_and_reloads_after_failures_and_channel_changes() {
    gtk_test(
        "ui::settings::tests::release_picker::the_menu_lists_its_channel_s_releases_and_reloads_after_failures_and_channel_changes",
        || {
            let manager = PreferenceManager::shared();
            manager.set_release_channel(Channel::Stable);
            let releases = FakeReleases::default();
            let (_window, picker) = open_picker(&releases);
            assert!(
                releases.requested().is_empty(),
                "Settings must not call GitHub before the menu opens"
            );

            picker.button.popup();
            assert_eq!(page_text(picker.menu.upcast_ref()), "Loading releases…");
            releases.answer(Err("GitHub API rate limit reached".to_owned()));
            wait_until("the failure message", || {
                page_text(picker.menu.upcast_ref()).starts_with("Couldn't load releases")
            });
            picker.button.popdown();
            picker.button.popup();
            assert_eq!(releases.requested(), [Channel::Stable, Channel::Stable]);

            let installed = crate::build_info::installed_version().to_string();
            releases.answer(Ok(vec![
                choice(&installed, BuildKind::Stable),
                choice("0.4.0", BuildKind::Stable),
            ]));
            wait_until("the release options", || options(&picker).len() == 2);
            assert_eq!(
                page_text(option(&picker, "0.4.0").upcast_ref()).trim_end(),
                "v0.4.0\nStable · Sep 20, 2026"
            );
            let marked = |version: &str| {
                descendants(option(&picker, version).upcast_ref())
                    .into_iter()
                    .any(|widget| widget.is::<gtk::Image>() && widget.get_visible())
            };
            assert!(marked(&installed), "the installed release is checked");
            assert!(!marked("0.4.0"));

            picker.button.popdown();
            picker.button.popup();
            assert_eq!(releases.requested().len(), 2, "a listed channel is reused");

            manager.set_release_channel(Channel::Nightly);
            assert_eq!(
                releases.requested(),
                [Channel::Stable, Channel::Stable, Channel::Nightly]
            );
            assert_eq!(page_text(picker.menu.upcast_ref()), "Loading releases…");
            releases.answer(Ok(Vec::new()));
            wait_until("the empty channel message", || {
                page_text(picker.menu.upcast_ref()) == "No releases are available on this channel."
            });
            manager.set_release_channel(Channel::Stable);
        },
    );
}

#[test]
fn choosing_an_older_release_confirms_it_then_installs_exactly_that_release() {
    gtk_test(
        "ui::settings::tests::release_picker::choosing_an_older_release_confirms_it_then_installs_exactly_that_release",
        || {
            PreferenceManager::shared().set_release_channel(Channel::Stable);
            let releases = FakeReleases::default();
            let (window, picker) = open_picker(&releases);
            let installed = crate::build_info::installed_version().to_string();
            picker.button.popup();
            let older = choice("0.4.0", BuildKind::Stable);
            releases.answer(Ok(vec![
                choice(&installed, BuildKind::Stable),
                older.clone(),
            ]));
            wait_until("the release options", || options(&picker).len() == 2);

            option(&picker, &installed).emit_clicked();
            assert!(button_labelled(&window, "Install release").is_none());

            option(&picker, "0.4.0").emit_clicked();
            wait_until("the confirmation dialog", || {
                button_labelled(&window, "Install release").is_some()
            });
            let text = page_text(window.upcast_ref());
            for expected in [
                "Install Strata v0.4.0".to_owned(),
                format!("Installed v{installed}  →  Selected v0.4.0"),
                "0123abcd".to_owned(),
                "Review the release notes before installing this release.".to_owned(),
            ] {
                assert!(
                    text.contains(&expected),
                    "{expected:?} missing from {text:?}"
                );
            }
            assert!(releases.installer.requests().is_empty());

            button_labelled(&window, "Install release")
                .expect("install action")
                .emit_clicked();

            assert_eq!(releases.installer.requests(), [older.install]);
            releases.installer.report(UpdateInstall::Installed);
            wait_until("the installed status", || {
                page_text(window.upcast_ref()).contains("Release installed — restart to apply")
            });
        },
    );
}

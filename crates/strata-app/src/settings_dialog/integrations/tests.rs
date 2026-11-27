// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    fs,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use rune::{Key, Mods};
use serde_json::Value;
use strata_core::i18n::tr;
use strata_desktop::services::{
    integration_catalog::{Entry, RequirementResult, parse},
    package_manager::PackageManager,
};

use super::{Checker, Source};
use crate::{
    clipboard,
    host::offscreen::Offscreen,
    settings_dialog::tests::{click, has_state, press},
    test_support::{
        PATIENCE, hosted,
        view::{self, idle},
    },
};

const MODULE: &str = "settings_dialog::integrations::tests";

fn entry(id: &str, name: &str, category: &str, script: &[u8]) -> Entry {
    let manifest = format!(
        r#"
schema_version = 1
id = "{id}"
name = "{name}"
description = "Share links for {name}."
category = "{category}"
maintainer = "octocat"
runtime = "sh"
entry = "provider.sh"

[[requirements]]
command = "example-daemon"
packages = {{ pacman = "example-pkg" }}
"#
    );
    let files = [
        ("catalog.toml", manifest.into_bytes()),
        ("provider.sh", script.to_vec()),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_owned(), bytes))
    .collect();
    parse(id, files).expect("valid entry")
}

fn example(script: &[u8]) -> Entry {
    entry("example", "Example Cloud", "cloud", script)
}

fn checker(checks: &Arc<AtomicUsize>) -> Checker {
    let checks = Arc::clone(checks);
    Arc::new(move |entry| {
        checks.fetch_add(1, Ordering::SeqCst);
        entry
            .requirements()
            .into_iter()
            .map(|requirement| RequirementResult {
                satisfied: requirement.command.as_deref() == Some("sh"),
                requirement,
            })
            .collect()
    })
}

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn open_integrations(host: &mut Offscreen) {
    click(host, "header-settings");
    click(host, "settings-nav-integrations");
}

fn revisit_integrations(host: &mut Offscreen) {
    click(host, "settings-nav-general");
    click(host, "settings-nav-integrations");
}

#[test]
fn catalog_installs_toggles_updates_removes_and_discovers_manual_integrations() {
    hosted(
        MODULE,
        "catalog_installs_toggles_updates_removes_and_discovers_manual_integrations",
        |mut host| {
            let config = tempfile::tempdir().expect("config");
            let root = config.path().join("strata/providers");
            let catalog = Arc::new(Mutex::new(vec![example(b"#!/bin/sh\ncat\n")]));
            let reloads = Rc::new(Cell::new(0usize));
            let checks = Arc::new(AtomicUsize::new(0));

            host.app_mut().settings_dialog.integrations.source = Source {
                catalog: Arc::new({
                    let catalog = Arc::clone(&catalog);
                    move || catalog.lock().expect("catalog").clone()
                }),
                root: Some(root.clone()),
                check: checker(&checks),
                detect_manager: Arc::new(|| Some(PackageManager::Pacman)),
                reload: Some(Rc::new({
                    let reloads = Rc::clone(&reloads);
                    move |_| reloads.set(reloads.get() + 1)
                })),
                open_folder: None,
            };

            open_integrations(&mut host);
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app()
                        .settings_dialog
                        .integrations
                        .requirements
                        .contains_key("example")
                }),
                "catalog and requirements probed off-thread"
            );
            idle(&mut host);

            let tree = host.tree();
            assert!(find_by_key(&tree, "integration-example-card").is_some());
            assert!(find_by_key(&tree, "integration-example-install").is_some());
            assert!(
                find_by_key(&tree, "integration-example").is_none(),
                "nothing is listed as installed yet"
            );
            let req_toggle =
                find_by_key(&tree, "integration-example-requirements").expect("requirements");
            assert_eq!(
                view::text(req_toggle, "description"),
                tr("Some requirements are missing.")
            );
            assert_eq!(reloads.get(), 0, "no reload before a change");

            // Expand requirements, verify chips and copyable install command feedback timer.
            click(&mut host, "integration-example-requirements");
            let tree = host.tree();
            assert!(find_by_key(&tree, "integration-example-requirements-details").is_some());
            let copy_btn =
                find_by_key(&tree, "integration-example-copy-cmd-0").expect("copy cmd button");
            assert_eq!(view::text(copy_btn, "name"), tr("Copy install command"));

            click(&mut host, "integration-example-copy-cmd-0");
            assert_eq!(
                clipboard::fallback_text(host.app()),
                Some("sudo pacman -S --needed example-pkg")
            );
            let tree = host.tree();
            let copy_btn =
                find_by_key(&tree, "integration-example-copy-cmd-0").expect("copy cmd button");
            assert_eq!(view::text(copy_btn, "name"), tr("Install command copied"));

            host.runtime().frame(0.5);
            idle(&mut host);
            let tree = host.tree();
            let copy_btn =
                find_by_key(&tree, "integration-example-copy-cmd-0").expect("copy cmd button");
            assert_eq!(view::text(copy_btn, "name"), tr("Install command copied"));

            host.runtime().frame(2.0);
            idle(&mut host);
            let tree = host.tree();
            let copy_btn =
                find_by_key(&tree, "integration-example-copy-cmd-0").expect("copy cmd button");
            assert_eq!(view::text(copy_btn, "name"), tr("Copy install command"));

            // Revisiting the page reuses requirement results and keeps requirements expanded.
            revisit_integrations(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example-card").is_some()
            }));
            assert_eq!(
                checks.load(Ordering::SeqCst),
                1,
                "revisiting reuses cached requirements"
            );
            assert!(
                find_by_key(&host.tree(), "integration-example-requirements-details").is_some(),
                "expanded requirements stay open across revisits"
            );

            // Install from catalog.
            click(&mut host, "integration-example-install");
            assert!(
                host.run_until(PATIENCE, |h| {
                    find_by_key(&h.tree(), "integration-example").is_some()
                }),
                "installed row appears"
            );
            idle(&mut host);
            assert!(root.join("example/provider.json").exists());
            assert_eq!(reloads.get(), 1, "install reloads running providers");
            let tree = host.tree();
            assert!(find_by_key(&tree, "integration-example-install").is_none());
            assert!(find_by_key(&tree, "integration-example-installed").is_some());
            assert!(find_by_key(&tree, "integration-example-requirements-details").is_some());
            let origin = find_by_key(&tree, "integration-example-origin").expect("origin");
            assert_eq!(view::text(origin, "name"), tr("Installed via catalog"));

            // Toggle enabled -> disabled -> enabled.
            let switch = find_by_key(&tree, "integration-example-enabled").expect("switch");
            assert!(has_state(switch, "checked"));
            click(&mut host, "integration-example-enabled");
            assert!(host.run_until(PATIENCE, |h| {
                reloads.get() == 2
                    && find_by_key(&h.tree(), "integration-example-enabled")
                        .is_some_and(|n| !has_state(n, "checked"))
            }));
            assert!(root.join("example/disabled").exists());
            click(&mut host, "integration-example-enabled");
            assert!(host.run_until(PATIENCE, |h| {
                reloads.get() == 3
                    && find_by_key(&h.tree(), "integration-example-enabled")
                        .is_some_and(|n| has_state(n, "checked"))
            }));
            assert!(!root.join("example/disabled").exists());

            // Update with local edits requires inline confirmation before replacing.
            *catalog.lock().expect("catalog") = vec![example(b"#!/bin/sh\nexec cat\n")];
            fs::write(root.join("example/provider.sh"), b"edited").expect("edit");
            revisit_integrations(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example-update").is_some()
            }));
            click(&mut host, "integration-example-update");
            assert!(find_by_key(&host.tree(), "integration-example-confirm").is_some());
            assert_eq!(
                fs::read(root.join("example/provider.sh")).expect("script"),
                b"edited",
                "local edit is confirmed before replacing"
            );
            click(&mut host, "integration-example-cancel");
            assert!(find_by_key(&host.tree(), "integration-example-confirm").is_none());
            click(&mut host, "integration-example-update");
            click(&mut host, "integration-example-confirm");
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example").is_some()
                    && find_by_key(&h.tree(), "integration-example-update").is_none()
            }));
            assert_eq!(
                fs::read(root.join("example/provider.sh")).expect("script"),
                b"#!/bin/sh\nexec cat\n"
            );

            // Remove unmodified catalog integration immediately.
            click(&mut host, "integration-example-remove");
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example-install").is_some()
            }));
            assert!(find_by_key(&host.tree(), "integration-example").is_none());
            assert!(!root.join("example").exists());

            // Manual installation appears on Refresh and confirms before removal.
            fs::create_dir_all(root.join("handmade")).expect("manual provider");
            fs::write(
                root.join("handmade/provider.json"),
                br#"{"version":1,"id":"handmade","name":"Handmade","command":["/bin/sh"]}"#,
            )
            .expect("manifest");
            let before_reloads = reloads.get();
            click(&mut host, "integrations-refresh");
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-handmade").is_some()
                    && checks.load(Ordering::SeqCst) == 2
            }));
            assert_eq!(reloads.get(), before_reloads + 1);
            let tree = host.tree();
            let origin = find_by_key(&tree, "integration-handmade-origin").expect("manual origin");
            assert_eq!(view::text(origin, "name"), tr("Manually installed"));
            click(&mut host, "integration-handmade-remove");
            assert!(find_by_key(&host.tree(), "integration-handmade-confirm").is_some());
            assert!(root.join("handmade").exists());
            click(&mut host, "integration-handmade-confirm");
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-handmade").is_none()
            }));
            assert!(!root.join("handmade").exists());
        },
    );
}

#[test]
fn category_filters_and_empty_catalog() {
    hosted(MODULE, "category_filters_and_empty_catalog", |mut host| {
        let config = tempfile::tempdir().expect("config");
        let root = config.path().join("providers");
        let checks = Arc::new(AtomicUsize::new(0));
        let catalog = Arc::new(Mutex::new(vec![
            example(b"#!/bin/sh\n"),
            entry("phone", "Phone Link", "devices", b"#!/bin/sh\n"),
        ]));
        host.app_mut().settings_dialog.integrations.source = Source {
            catalog: Arc::new({
                let catalog = Arc::clone(&catalog);
                move || catalog.lock().expect("catalog").clone()
            }),
            root: Some(root),
            check: checker(&checks),
            detect_manager: Arc::new(|| None),
            reload: Some(Rc::new(|_| {})),
            open_folder: None,
        };

        open_integrations(&mut host);
        assert!(host.run_until(PATIENCE, |h| {
            let tree = h.tree();
            find_by_key(&tree, "integration-example-card").is_some()
                && find_by_key(&tree, "integration-phone-card").is_some()
        }));
        assert!(find_by_key(&host.tree(), "integrations-filters").is_some());

        click(&mut host, "integration-filter-devices");
        let tree = host.tree();
        assert!(find_by_key(&tree, "integration-phone-card").is_some());
        assert!(find_by_key(&tree, "integration-example-card").is_none());

        click(&mut host, "integration-filter-all");
        let tree = host.tree();
        assert!(find_by_key(&tree, "integration-phone-card").is_some());
        assert!(find_by_key(&tree, "integration-example-card").is_some());

        // Empty catalog explains itself and keeps Refresh available.
        catalog.lock().expect("catalog").clear();
        click(&mut host, "integrations-refresh");
        assert!(host.run_until(PATIENCE, |h| {
            find_by_key(&h.tree(), "integrations-empty").is_some()
        }));
        let tree = host.tree();
        let empty = find_by_key(&tree, "integrations-empty").expect("empty label");
        assert_eq!(
            view::text(empty, "name"),
            tr("No integrations are bundled with this build.")
        );
        assert!(find_by_key(&tree, "integrations-refresh").is_some());
        assert!(find_by_key(&tree, "integrations-filters").is_none());
    });
}

#[test]
fn open_folder_closes_settings_and_navigates_to_providers_root() {
    hosted(
        MODULE,
        "open_folder_closes_settings_and_navigates_to_providers_root",
        |mut host| {
            let dir = tempfile::tempdir().expect("providers dir");
            let root = dir.path().to_path_buf();
            let opened = Rc::new(Mutex::new(None));
            host.app_mut().settings_dialog.integrations.source = Source {
                catalog: Arc::new(Vec::new),
                root: Some(root.clone()),
                check: Arc::new(|_| Vec::new()),
                detect_manager: Arc::new(|| None),
                reload: Some(Rc::new(|_| {})),
                open_folder: Some(Rc::new({
                    let opened = Rc::clone(&opened);
                    move |app, path| {
                        crate::settings_dialog::close_settings(app);
                        *opened.lock().expect("opened") = Some(path);
                    }
                })),
            };

            open_integrations(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integrations-open-folder").is_some()
            }));
            click(&mut host, "integrations-open-folder");
            assert!(!host.app().settings_dialog.is_open());
            assert_eq!(*opened.lock().expect("opened"), Some(root));
        },
    );
}

#[test]
fn settings_search_reveals_integrations_page() {
    hosted(
        MODULE,
        "settings_search_reveals_integrations_page",
        |mut host| {
            host.app_mut().settings_dialog.integrations.source = Source {
                catalog: Arc::new(Vec::new),
                root: None,
                check: Arc::new(|_| Vec::new()),
                detect_manager: Arc::new(|| None),
                reload: Some(Rc::new(|_| {})),
                open_folder: None,
            };

            click(&mut host, "header-settings");
            click(&mut host, "settings-search-field");
            for ch in "localsend".chars() {
                press(&mut host, Key::Text(ch.to_string()), Mods::default());
            }
            host.runtime().frame(0.2);
            idle(&mut host);

            assert!(
                find_by_key(&host.tree(), "integrations-refresh").is_some(),
                "searching localsend navigates to and displays the Integrations page"
            );
        },
    );
}

#[test]
fn closing_settings_discards_in_flight_scan_results() {
    hosted(
        MODULE,
        "closing_settings_discards_in_flight_scan_results",
        |mut host| {
            let config = tempfile::tempdir().expect("config");
            let (started_tx, started_rx) = mpsc::channel::<()>();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            let release_rx = Arc::new(Mutex::new(Some(release_rx)));
            let started_tx = Arc::new(Mutex::new(Some(started_tx)));

            host.app_mut().settings_dialog.integrations.source = Source {
                catalog: Arc::new(move || {
                    if let Some(tx) = started_tx.lock().expect("started").take() {
                        let _ = tx.send(());
                    }
                    if let Some(rx) = release_rx.lock().expect("release").take() {
                        let _ = rx.recv();
                    }
                    vec![example(b"#!/bin/sh\n")]
                }),
                root: Some(config.path().join("providers")),
                check: Arc::new(|_| Vec::new()),
                detect_manager: Arc::new(|| Some(PackageManager::Pacman)),
                reload: Some(Rc::new(|_| {})),
                open_folder: None,
            };

            open_integrations(&mut host);
            started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("background scan started");
            let old_gen = host.app().settings_dialog.integrations.scan_generation;

            // Close Settings while the off-thread scan is still running.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            assert!(host.app().settings_dialog.integrations.scan_generation > old_gen);

            // Release the stale worker and pump the loop; stale payload must be discarded.
            release_tx.send(()).expect("release worker");
            host.wait(Duration::from_millis(120));
            idle(&mut host);

            let state = &host.app().settings_dialog.integrations;
            assert!(!state.loaded);
            assert!(state.rows.is_empty());
            assert!(state.requirements.is_empty());
        },
    );
}

#[test]
fn stale_mutation_keeps_new_busy_lock_and_refresh_mid_mutation_rescans() {
    hosted(
        MODULE,
        "stale_mutation_keeps_new_busy_lock_and_refresh_mid_mutation_rescans",
        |mut host| {
            let config = tempfile::tempdir().expect("config");
            let root = config.path().join("strata/providers");
            let entry_data = example(b"#!/bin/sh\ncat\n");
            let reloads = Rc::new(Cell::new(0usize));

            host.app_mut().settings_dialog.integrations.source = Source {
                catalog: Arc::new({
                    let entry_data = entry_data.clone();
                    move || vec![entry_data.clone()]
                }),
                root: Some(root.clone()),
                check: Arc::new(|_| Vec::new()),
                detect_manager: Arc::new(|| None),
                reload: Some(Rc::new({
                    let reloads = Rc::clone(&reloads);
                    move |_| reloads.set(reloads.get() + 1)
                })),
                open_folder: None,
            };

            open_integrations(&mut host);
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example-install").is_some()
            }));

            let (stale_tx, stale_rx) = mpsc::channel::<()>();
            super::probe::run_mutation(host.app_mut(), "example".to_owned(), move || {
                let _ = stale_rx.recv();
                Err("stale failure".to_owned())
            });
            let stale_token = host
                .app()
                .settings_dialog
                .integrations
                .busy
                .get("example")
                .copied()
                .expect("stale busy token");

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            assert!(host.app().settings_dialog.integrations.busy.is_empty());

            click(&mut host, "header-settings");
            assert!(host.run_until(PATIENCE, |h| {
                find_by_key(&h.tree(), "integration-example-install").is_some()
            }));
            let (fresh_tx, fresh_rx) = mpsc::channel::<()>();
            super::probe::run_mutation(host.app_mut(), "example".to_owned(), move || {
                let _ = fresh_rx.recv();
                Ok(())
            });
            let fresh_token = host
                .app()
                .settings_dialog
                .integrations
                .busy
                .get("example")
                .copied()
                .expect("fresh busy token");
            assert_ne!(fresh_token, stale_token);

            stale_tx.send(()).expect("release stale mutation");
            host.wait(Duration::from_millis(120));
            idle(&mut host);

            let state = &host.app().settings_dialog.integrations;
            assert_eq!(
                state.busy.get("example").copied(),
                Some(fresh_token),
                "stale mutation must not unlock a newer mutation on the same id"
            );
            assert!(
                !state.row_messages.contains_key("example"),
                "stale mutation error must not surface in the newer session"
            );

            fresh_tx.send(()).expect("release fresh mutation");
            assert!(host.run_until(PATIENCE, |h| {
                !h.app()
                    .settings_dialog
                    .integrations
                    .busy
                    .contains_key("example")
            }));

            let (install_tx, install_rx) = mpsc::channel::<()>();
            let install_entry = entry_data.clone();
            let install_root = root.clone();
            super::probe::run_mutation(host.app_mut(), "example".to_owned(), move || {
                let _ = install_rx.recv();
                strata_desktop::services::integration_catalog::install(
                    &install_entry,
                    &install_root,
                )
                .map(|_| ())
            });
            let inflight_token = host
                .app()
                .settings_dialog
                .integrations
                .busy
                .get("example")
                .copied()
                .expect("in-flight busy token");

            click(&mut host, "integrations-refresh");
            assert_eq!(
                host.app()
                    .settings_dialog
                    .integrations
                    .busy
                    .get("example")
                    .copied(),
                Some(inflight_token),
                "refresh preserves in-flight mutation busy lock"
            );

            install_tx.send(()).expect("finish in-flight install");
            assert!(
                host.run_until(PATIENCE, |h| {
                    find_by_key(&h.tree(), "integration-example").is_some()
                        && !h
                            .app()
                            .settings_dialog
                            .integrations
                            .busy
                            .contains_key("example")
                }),
                "completing a mutation after mid-flight refresh rescans and unlocks the row"
            );
        },
    );
}

// SPDX-License-Identifier: MIT

use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

fn find(root: &gtk::Widget, predicate: &impl Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if predicate(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

fn status(form: &EditorForm, text: &str) {
    let context = gtk::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    assert!(
        find(form.root.upcast_ref(), &|widget| widget.is_mapped()
            && widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == text))
        .is_some(),
        "missing runtime status: {text}"
    );
}

#[test]
fn runtime_readiness_tracks_shebangs_installs_and_runtime_switches_without_execution() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::readiness::runtime_readiness_tracks_shebangs_installs_and_runtime_switches_without_execution",
        || {
            let directory = tempfile::tempdir().expect("runtime fixture");
            let interpreter = directory.path().join("python3");
            let executed = directory.path().join("must-not-execute");
            let form = form_for(python_draft(), None);
            form.script
                .buffer()
                .set_text(&format!("#!{}\nprint('draft')\n", interpreter.display()));
            form.tabs.set_current_page(Some(SCRIPT_TAB));
            let window = gtk::Window::builder().child(&form.root).build();
            window.present();
            status(&form, "Python not found — cannot run");
            window.set_visible(false);
            fs::write(
                &interpreter,
                format!("#!/bin/sh\n: > '{}'\nexit 7\n", executed.display()),
            )
            .expect("fixture executable");
            fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o600))
                .expect("not executable");
            window.present();
            status(&form, "Python not found — cannot run");
            window.set_visible(false);
            fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o700))
                .expect("executable");
            window.present();
            status(&form, "Python available");
            choose(&form, "Command");
            form.program
                .set_text(interpreter.to_str().expect("fixture path"));
            window.set_visible(false);
            fs::remove_file(&interpreter).expect("remove runtime");
            window.present();
            choose(&form, "Python");
            status(&form, "Python not found — cannot run");
            form.script
                .buffer()
                .set_text("#!/bin/bash\nprintf 'wrong runtime'\n");
            status(&form, "Invalid interpreter — cannot run");
            choose(&form, "Bash");
            status(&form, "Bash available");
            choose(&form, "Python");
            status(&form, "Invalid interpreter — cannot run");
            assert!(
                !executed.exists(),
                "availability never executes a program or draft"
            );
            window.close();
        },
    );
}

// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use gtk::glib;

use super::{InstallCancel, InstallLauncher, InstallRequest, UpdateInstall};

mod general;
mod restart;
mod updates;

const GUARD_REJECTION: &str = "Another install is already running — try again shortly.";

fn offered_request() -> InstallRequest {
    InstallRequest {
        tag: "v99.0.0".to_owned(),
        asset_name: format!(
            "strata-99.0.0-{}-unknown-linux-gnu.tar.gz",
            std::env::consts::ARCH
        ),
        advertised_url: "http://127.0.0.1:9/strata.tar.gz".to_owned(),
    }
}

/// Stands in for [`crate::services::install_update`]: records each launch and
/// lets the test report the install's progress and outcome.
#[derive(Default)]
struct FakeInstaller {
    launches: Rc<RefCell<Vec<(InstallRequest, InstallCancel)>>>,
    progress: Rc<RefCell<Option<mpsc::Sender<UpdateInstall>>>>,
}

impl FakeInstaller {
    fn launcher(&self) -> InstallLauncher {
        let launches = self.launches.clone();
        let progress = self.progress.clone();
        Rc::new(move |request, cancel| {
            launches.borrow_mut().push((request, cancel));
            let (sender, receiver) = mpsc::channel();
            progress.replace(Some(sender));
            receiver
        })
    }

    fn requests(&self) -> Vec<InstallRequest> {
        self.launches
            .borrow()
            .iter()
            .map(|(request, _)| request.clone())
            .collect()
    }

    fn report(&self, event: UpdateInstall) {
        self.progress
            .borrow()
            .as_ref()
            .expect("a started install")
            .send(event)
            .expect("the dialog or row is still listening");
    }
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

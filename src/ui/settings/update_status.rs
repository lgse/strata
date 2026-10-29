// SPDX-License-Identifier: MIT

use super::*;
use crate::services::Listeners;

struct CompletedCheck {
    channel: Channel,
    method: UpdateMethod,
    result: UpdateCheck,
}

type Handler = Rc<dyn Fn(Option<&CompletedCheck>)>;

thread_local! {
    static CURRENT: RefCell<Option<Rc<CompletedCheck>>> = const { RefCell::new(None) };
    static HANDLERS: Rc<Listeners<Handler>> = Rc::new(Listeners::new());
}

pub(super) fn publish(channel: Channel, method: UpdateMethod, result: UpdateCheck) {
    let check = Rc::new(CompletedCheck {
        channel,
        method,
        result,
    });
    CURRENT.replace(Some(check.clone()));
    HANDLERS.with(|handlers| handlers.notify(|handler| handler(Some(&check))));
}

pub(super) fn clear() {
    CURRENT.take();
    HANDLERS.with(|handlers| handlers.notify(|handler| handler(None)));
}

/// Rows keep their last rendering until the next result lands.
pub(super) fn forget() {
    CURRENT.take();
}

pub(super) fn available() -> CachedUpdate {
    let current = CURRENT.with_borrow(Clone::clone)?;
    match &current.result {
        UpdateCheck::Available { release, install } => {
            Some((release.clone(), install.clone(), current.method))
        }
        UpdateCheck::UpToDate | UpdateCheck::Failed(_) => None,
    }
}

pub(super) struct View {
    pub title: gtk::Label,
    pub icon: gtk::Image,
    pub status: gtk::Label,
    pub button: gtk::Button,
    pub notes: ReleaseNotesCard,
    pub pending: Rc<RefCell<Option<PendingInstall>>>,
    pub managed: Rc<Cell<bool>>,
    pub checking: Rc<Cell<bool>>,
    pub install_underway: Rc<dyn Fn() -> bool>,
    pub manager: Rc<PreferenceManager>,
    pub method: UpdateMethod,
}

impl View {
    pub(super) fn bind(self, anchor: &gtk::Box) {
        let handler: Handler = Rc::new(move |check| self.apply(check));
        let guard = RefCell::new(Some(
            HANDLERS.with(|handlers| handlers.add(handler.clone())),
        ));
        anchor.connect_destroy(move |_| {
            guard.take();
        });
        let current = CURRENT.with_borrow(Clone::clone);
        handler(current.as_deref());
    }

    fn apply(&self, check: Option<&CompletedCheck>) {
        if (self.install_underway)() {
            return;
        }
        let check = check.filter(|check| {
            check.method == self.method
                && check.channel
                    == effective_update_channel(self.manager.release_channel(), self.method)
        });
        let Some(check) = check else {
            if self.checking.get() {
                return;
            }
            self.pending.take();
            self.managed.set(false);
            self.title.set_text(&crate::i18n::tr("Check for updates"));
            self.status.set_text(&installed_version_status(
                &crate::build_info::installed_version(),
                crate::build_info::build_kind(),
                self.method,
            ));
            crate::assets::set_primary_icon(&self.icon, icons::INFO);
            self.button.set_label(&crate::i18n::tr("Check now"));
            self.button.set_sensitive(true);
            self.notes.container.set_visible(false);
            return;
        };
        let result = &check.result;
        crate::assets::set_primary_icon(
            &self.icon,
            match result {
                UpdateCheck::UpToDate => icons::CIRCLE_CHECK,
                UpdateCheck::Available { .. } => icons::DOWNLOADS,
                UpdateCheck::Failed(_) => icons::TRIANGLE_ALERT,
            },
        );
        self.title.set_text(&crate::i18n::tr(match result {
            UpdateCheck::UpToDate => "Strata is up to date",
            UpdateCheck::Available { .. } => "An update is available",
            UpdateCheck::Failed(_) => "Couldn’t check for updates",
        }));
        let returns_to_stable = matches!(result,
            UpdateCheck::Available { release, .. }
                if check.channel == Channel::Stable
                    && crate::build_info::build_kind() != BuildKind::Stable
                    && release.kind == BuildKind::Stable
        );
        let message = if returns_to_stable
            && matches!(self.method, UpdateMethod::InPlace | UpdateMethod::Aur)
        {
            let UpdateCheck::Available { release, .. } = result else {
                unreachable!();
            };
            rust_i18n::t!(
                "Stable channel target: <a href=\"%{value1}\">v%{value2}</a>",
                value1 = glib::markup_escape_text(&release.url),
                value2 = glib::markup_escape_text(&release.version)
            )
            .into_owned()
        } else {
            update_check_message(result, self.method)
        };
        self.status.set_markup(&update_status_markup(
            message,
            result,
            InstallSource::detect(),
        ));
        self.pending.take();
        self.managed.set(false);
        self.button.set_label(&crate::i18n::tr("Check now"));
        self.notes
            .container
            .set_visible(shows_available_release_notes(result));
        if let UpdateCheck::Available { release, install } = result {
            show_release_notes(&self.notes, release);
            if self.method.is_package_managed() {
                self.managed.set(true);
                self.button.set_label(&crate::i18n::tr(match self.method {
                    UpdateMethod::Aur => aur_update_action_label(),
                    UpdateMethod::Omarchy => "Open Omarchy Update",
                    UpdateMethod::Pacman => "Check again",
                    UpdateMethod::InPlace => unreachable!(),
                }));
            } else {
                self.pending.replace(Some(PendingInstall {
                    kind: release.kind,
                    returns_to_stable,
                    request: install.clone(),
                }));
                self.button
                    .set_label(&crate::i18n::tr(if returns_to_stable {
                        "Return to stable"
                    } else {
                        "Install update"
                    }));
            }
        }
        self.button.set_sensitive(true);
        self.checking.set(false);
    }
}

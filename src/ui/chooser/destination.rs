// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct DestinationLease(glib::WeakRef<gtk::Window>);

impl DestinationLease {
    pub(super) fn acquire(parent: &gtk::Window) -> Option<Self> {
        let acquired = DESTINATION_PARENTS.with(|parents| {
            let mut parents = parents.borrow_mut();
            parents.retain(|parent| parent.upgrade().is_some());
            if parents
                .iter()
                .any(|candidate| candidate.upgrade().as_ref() == Some(parent))
            {
                return false;
            }
            parents.push(parent.downgrade());
            true
        });
        if acquired {
            Some(Self(parent.downgrade()))
        } else {
            let active = CHOOSERS.with(|choosers| {
                choosers
                    .borrow()
                    .values()
                    .filter_map(glib::WeakRef::upgrade)
                    .find(|window| window.transient_for().as_ref() == Some(parent))
            });
            if let Some(window) = active {
                window.present();
            }
            None
        }
    }
}

impl Drop for DestinationLease {
    fn drop(&mut self) {
        DESTINATION_PARENTS.with(|parents| {
            parents.borrow_mut().retain(|parent| {
                parent.upgrade() != self.0.upgrade() && parent.upgrade().is_some()
            });
        });
    }
}

pub(crate) struct DestinationRequest {
    pub parent: gtk::Window,
    pub title: String,
    pub accept_label: String,
    pub initial_directory: PathBuf,
    pub root_limit: Option<PathBuf>,
    pub allow_create: bool,
    pub validate: DestinationValidator,
}

pub(crate) fn present_destination_chooser(
    destination: DestinationRequest,
    completion: impl FnOnce(PathBuf) + 'static,
) {
    let Some(lease) = DestinationLease::acquire(&destination.parent) else {
        return;
    };
    let request = ChooserRequest {
        token: glib::uuid_string_random().to_string(),
        title: destination.title,
        accept_label: destination.accept_label,
        modal: true,
        parent: None,
        parent_size_hint: Some((destination.parent.width(), destination.parent.height())),
        initial_directory: destination.initial_directory,
        kind: ChooserKind::Open {
            directory: true,
            multiple: false,
        },
        filters: Vec::new(),
        current_filter: None,
        choices: Vec::new(),
    };
    let source = Rc::new(ChooserFileSource {
        root_limit: destination.root_limit.clone(),
        source: Rc::new(LocalFileSource),
        filter: Rc::new(RefCell::new(None)),
        directory_only: Rc::new(Cell::new(true)),
    });
    let host = DestinationHost {
        parent: destination.parent,
        validate: destination.validate,
        allow_create: destination.allow_create,
        confined: destination.root_limit.is_some(),
    };
    glib::MainContext::default().spawn_local(async move {
        crate::portal::prepare_chooser_placement().await;
        if !host.parent.is_visible() {
            return;
        }
        build_chooser_hosted(
            request,
            Arc::new(AtomicBool::new(false)),
            move |result| {
                drop(lease);
                if let Ok(result) = result
                    && let Some(path) = result
                        .uris()
                        .first()
                        .and_then(|uri| gio::File::for_uri(uri.as_str()).path())
                {
                    completion(path);
                }
            },
            source,
            Some(host),
        );
    });
}

pub(super) struct DestinationHost {
    pub(super) parent: gtk::Window,
    pub(super) validate: DestinationValidator,
    pub(super) allow_create: bool,
    pub(super) confined: bool,
}

// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    sandbox::Cancellation,
    services::image_conversion::{self, ImageKind},
};

impl ChooserState {
    pub(super) fn image_target(&self, path: &Path, kind: ImageKind) -> Option<PathBuf> {
        let filter = self
            .filter_dropdown
            .as_ref()
            .and_then(|dropdown| self.filters.get(dropdown.selected()));
        let matches = |candidate: &Path| {
            let info = gio::FileInfo::new();
            info.set_name(Path::new(candidate.file_name().unwrap_or_default()));
            info.set_display_name(&candidate.file_name().unwrap_or_default().to_string_lossy());
            info.set_file_type(gio::FileType::Regular);
            info.set_content_type(kind.mime());
            filter.is_none_or(|filter| filter.native.match_(&info))
        };
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                kind.extensions()
                    .iter()
                    .any(|valid| ext.eq_ignore_ascii_case(valid))
            })
            && matches(path)
        {
            return Some(path.to_owned());
        }
        kind.extensions()
            .iter()
            .flat_map(|ext| [ext.to_string(), ext.to_ascii_uppercase()])
            .map(|extension| image_conversion::with_extension(path, &extension))
            .find(|candidate| matches(candidate))
    }

    pub(super) fn complete_remote(self: &Rc<Self>, path: PathBuf) {
        self.accept_button.set_sensitive(true);
        let Some(kind) = image_conversion::detect(&path) else {
            // A PNG-looking filename alone must not pass a PNG-only filter.
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
                && self.image_target(&path, ImageKind::Png).is_some()
                && self.image_target(&path, ImageKind::Jpeg).is_none()
            {
                self.show_error("This download is not a supported image. Choose a JPEG, BMP, static WebP, GIF or PNG image.");
            } else {
                self.finish_remote(path);
            }
            return;
        };
        if let Some(target) = self.image_target(&path, kind) {
            if path != target
                && let Err(error) = std::fs::rename(&path, &target)
            {
                self.show_error(&format!("Could not name the downloaded image: {error}"));
                return;
            }
            self.finish_remote(target);
            return;
        }
        let Some(target) = self.image_target(&path, ImageKind::Png) else {
            self.show_error("The file does not match the selected filter");
            return;
        };
        let input = path.clone();
        self.image_job(
            "Checking image…",
            move |cancelled| image_conversion::inspect(&input, &cancelled),
            move |state, kind| {
                state.confirm_image_conversion(path.clone(), target.clone(), kind);
            },
        );
    }

    pub(super) fn image_job<T: Send + 'static>(
        self: &Rc<Self>,
        activity: &str,
        work: impl FnOnce(Cancellation) -> Result<T, String> + Send + 'static,
        done: impl Fn(&Rc<Self>, T) + 'static,
    ) {
        self.cancel_download();
        self.clear_error();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.download_cancel.replace(Some(cancelled.clone()));
        let weak = Rc::downgrade(self);
        self.show_download_progress(
            "",
            Rc::new(move || {
                if let Some(state) = weak.upgrade() {
                    state.cancel_download();
                }
            }),
        );
        if let Some(progress) = self.download_progress.borrow().as_ref() {
            progress.set_name("Image");
            progress.set_activity(activity);
        }
        self.accept_button.set_sensitive(false);
        let (sender, receiver) = std::sync::mpsc::channel();
        let token = Cancellation::from(cancelled.clone());
        std::thread::spawn(move || {
            let _ = sender.send(work(token));
        });
        let filter = self.selected_filter();
        let name = self.filename.as_ref().map(|entry| entry.text());
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(60), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state
                .download_cancel
                .borrow()
                .as_ref()
                .is_some_and(|flag| Arc::ptr_eq(flag, &cancelled))
            {
                return glib::ControlFlow::Break;
            }
            if state.selected_filter() != filter
                || state.filename.as_ref().map(|entry| entry.text()) != name
            {
                state.cancel_download();
                state.show_error("The name or filter changed. Press Open to try again.");
                return glib::ControlFlow::Break;
            }
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    Err("Image processing stopped unexpectedly. Try again.".into())
                }
            };
            state.download_cancel.take();
            state.dismiss_download_progress();
            state.accept_button.set_sensitive(true);
            match result {
                Ok(value) => done(&state, value),
                Err(message) => state.show_error(&message),
            }
            glib::ControlFlow::Break
        });
    }

    pub(super) fn confirm_image_conversion(
        self: &Rc<Self>,
        path: PathBuf,
        target: PathBuf,
        kind: ImageKind,
    ) {
        if self.completion.borrow().is_none() {
            return;
        }
        if visible_modal_layer(&self.window).is_some() {
            self.show_error("Close the current dialog, then press Open to convert the image.");
            return;
        }
        let Some(overlay) = self.window.child().and_downcast::<gtk::Overlay>() else {
            return;
        };
        let root = overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = root.as_ref() {
            root.set_blurred(true);
        }
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            "Convert image to PNG?",
            &format!(
                "This image is {}, which the selected filter does not accept. Convert it to PNG?",
                kind.label()
            ),
            "Convert to PNG",
            ModalTone::Accent,
        );
        let layer = modal_layer(&layout.content, &overlay, root.clone(), None);
        overlay.add_overlay(&layer);
        for button in [&layout.cancel, &layout.close] {
            let layer = layer.clone();
            let overlay = overlay.clone();
            let root = root.clone();
            button.connect_clicked(move |_| dismiss_modal_layer(&layer, &overlay, root.as_ref()));
        }
        let filter = self.selected_filter();
        let name = self.filename.as_ref().map(|entry| entry.text());
        let weak = Rc::downgrade(self);
        let confirmed_layer = layer.clone();
        layout.confirm.connect_clicked(move |_| {
            dismiss_modal_layer(&confirmed_layer, &overlay, root.as_ref());
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.completion.borrow().is_none() {
                return;
            }
            if state.selected_filter() != filter
                || state.filename.as_ref().map(|entry| entry.text()) != name
            {
                state.show_error("The name or filter changed. Press Open to try again.");
                return;
            }
            let path = path.clone();
            let target = target.clone();
            state.image_job(
                "Converting to PNG…",
                move |cancelled| image_conversion::convert(&path, &target, &cancelled),
                |state, path| state.finish_remote(path),
            );
        });
        let escape = gtk::EventControllerKey::new();
        let cancel = layout.cancel.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                cancel.emit_clicked();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        focus_button(&layout.cancel);
    }
}

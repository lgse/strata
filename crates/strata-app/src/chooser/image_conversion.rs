// SPDX-License-Identifier: MIT

//! PNG image conversion inspection, consent prompt, and worker execution
//! (`src/ui/chooser/image_conversion.rs`).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
    },
    time::Duration,
};

use rune::Tx;
use strata_core::i18n::tr;
use strata_preview::{
    sandbox::Cancellation,
    services::image_conversion::{self, ImageKind},
};

use super::{
    ChooserPrompt,
    download::{self, DownloadProgress},
};
use crate::{App, portal::PortalFileFilter, shell};

#[derive(Clone, Debug)]
pub(crate) struct ConversionConsent {
    pub(crate) path: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) kind: ImageKind,
    pub(crate) expected_filter: Option<PortalFileFilter>,
    pub(crate) expected_name: Option<String>,
}

pub(crate) fn image_target(app: &App, path: &Path, kind: ImageKind) -> Option<PathBuf> {
    let active = app.chooser.active.as_ref()?;
    let filter = active.selected_compiled_filter();
    let content_type = gio::content_type_from_mime_type(kind.mime())
        .map_or_else(|| kind.mime().to_owned(), String::from);
    let matches = |candidate: &Path| {
        let name = candidate
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        filter.is_none_or(|compiled| compiled.matches_file(&name, &content_type))
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
        .flat_map(|ext| [(*ext).to_owned(), ext.to_ascii_uppercase()])
        .map(|extension| image_conversion::with_extension(path, &extension))
        .find(|candidate| matches(candidate))
}

pub(crate) fn complete_download(app: &mut App, path: PathBuf) {
    let probe = path.clone();
    download::after_download_step(
        app,
        move || image_conversion::detect(&probe),
        move |app, kind, tx| classify_download(app, path, kind, tx),
    );
}

pub(crate) fn reuse_download(app: &mut App, url: String, path: PathBuf) {
    let probe = path.clone();
    download::after_download_step(
        app,
        move || probe.is_file().then(|| image_conversion::detect(&probe)),
        move |app, kept, tx| match kept {
            Some(kind) => classify_download(app, path, kind, tx),
            None => download::start_download(app, url, tx),
        },
    );
}

fn classify_download(app: &mut App, path: PathBuf, kind: Option<ImageKind>, tx: &mut Tx) {
    let Some(kind) = kind else {
        classify_non_image(app, path, tx);
        return;
    };
    if let Some(target) = image_target(app, &path, kind) {
        accept_supported_image(app, path, target, tx);
        return;
    }
    let Some(target) = image_target(app, &path, ImageKind::Png) else {
        download::finish_remote(app, path, tx);
        return;
    };
    let input = path.clone();
    image_job(
        app,
        "Checking image…",
        move |cancelled| image_conversion::inspect(&input, &cancelled),
        move |app, inspected_kind, _tx| {
            prompt_image_conversion(app, path.clone(), target.clone(), inspected_kind);
        },
    );
}

fn classify_non_image(app: &mut App, path: PathBuf, tx: &mut Tx) {
    // A PNG-looking filename alone must not pass a PNG-only filter.
    let fake_png = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        && image_target(app, &path, ImageKind::Png).is_some()
        && image_target(app, &path, ImageKind::Jpeg).is_none();
    if fake_png {
        app.chooser.show_error(tr(
                "This download is not a supported image. Choose a JPEG, BMP, static WebP, GIF or PNG image.",
            ));
    } else {
        download::finish_remote(app, path, tx);
    }
}

fn accept_supported_image(app: &mut App, path: PathBuf, target: PathBuf, tx: &mut Tx) {
    if path == target {
        download::finish_remote(app, target, tx);
        return;
    }
    let (from, to) = (path.clone(), target.clone());
    download::after_download_step(
        app,
        move || std::fs::rename(&from, &to),
        move |app, renamed, tx| match renamed {
            Ok(()) => {
                if let Some(active) = app.chooser.active.as_mut() {
                    active.rename_download(&path, &target);
                }
                download::finish_remote(app, target, tx);
            }
            Err(error) => {
                app.chooser.show_error(
                    rust_i18n::t!(
                        "Could not name the downloaded image: %{error}",
                        error = error
                    )
                    .into_owned(),
                );
            }
        },
    );
}

pub(crate) fn image_job<T: Send + 'static>(
    app: &mut App,
    activity: &str,
    work: impl FnOnce(Cancellation) -> Result<T, String> + Send + 'static,
    done: impl FnOnce(&mut App, T, &mut Tx) + 'static,
) {
    let later = app.chooser.later.clone();
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    active.cancel_download();
    active.clear_error();
    let cancelled = Arc::new(AtomicBool::new(false));
    active.download_cancel = Some(cancelled.clone());
    active.download_progress = Some(DownloadProgress::for_image(activity));
    active.accept_sensitive = false;
    let expected_filter = active.selected_filter();
    let expected_name = active.name_text();
    let (sender, receiver) = std::sync::mpsc::channel();
    let token = Cancellation::from(cancelled.clone());
    std::thread::spawn(move || {
        let _ = sender.send(work(token));
    });
    let mut done_opt = Some(done);
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let outcome = match receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "Image processing stopped unexpectedly. Try again.".into(),
            )),
        };
        if cancelled.load(Ordering::SeqCst) {
            return glib::ControlFlow::Break;
        }
        let Some(result) = outcome else {
            return glib::ControlFlow::Continue;
        };
        let Some(callback) = done_opt.take() else {
            return glib::ControlFlow::Break;
        };
        let flag = cancelled.clone();
        let filter_snapshot = expected_filter.clone();
        let name_snapshot = expected_name.clone();
        later.then(move |app: &mut App, tx: &mut Tx| {
            finish_image_job(
                app,
                (&flag, &filter_snapshot, &name_snapshot),
                result,
                (callback, tx),
            )
        });
        glib::ControlFlow::Break
    });
}

fn finish_image_job<T>(
    app: &mut App,
    (flag, expected_filter, expected_name): (
        &Arc<AtomicBool>,
        &Option<PortalFileFilter>,
        &Option<String>,
    ),
    result: Result<T, String>,
    (callback, tx): (impl FnOnce(&mut App, T, &mut Tx), &mut Tx),
) -> bool {
    let Some(active) = app.chooser.active.as_mut() else {
        return false;
    };
    if !active.download_is_current(flag) {
        return false;
    }
    if &active.selected_filter() != expected_filter || &active.name_text() != expected_name {
        active.cancel_download();
        active.show_error(tr("The name or filter changed. Press Open to try again."));
        return true;
    }
    active.download_cancel = None;
    active.download_progress = None;
    active.accept_sensitive = true;
    match result {
        Ok(value) => callback(app, value, tx),
        Err(message) => active.show_error(tr(&message)),
    }
    true
}

fn prompt_image_conversion(app: &mut App, path: PathBuf, target: PathBuf, kind: ImageKind) {
    let modal = shell::modal_open(app);
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    if active.completion.is_none() {
        return;
    }
    if modal {
        active.show_error(tr(
            "Close the current dialog, then press Open to convert the image.",
        ));
        return;
    }
    let expected_filter = active.selected_filter();
    let expected_name = active.name_text();
    app.chooser_dialog
        .open(ChooserPrompt::ConvertImage(ConversionConsent {
            path,
            target,
            kind,
            expected_filter,
            expected_name,
        }));
}

pub(crate) fn confirm_conversion(app: &mut App, consent: ConversionConsent) {
    let Some(active) = app.chooser.active.as_mut() else {
        return;
    };
    if active.completion.is_none() {
        return;
    }
    if active.selected_filter() != consent.expected_filter
        || active.name_text() != consent.expected_name
    {
        active.show_error(tr("The name or filter changed. Press Open to try again."));
        return;
    }
    let (path, target) = (consent.path, consent.target);
    image_job(
        app,
        "Converting to PNG…",
        move |cancelled| image_conversion::convert(&path, &target, &cancelled),
        download::finish_remote,
    );
}

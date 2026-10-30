// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use super::{
    PRINT_TEXT_BYTE_LIMIT, PreviewContent, PreviewEvent, PreviewRequest, PreviewRequestId,
    PreviewState, paint_print_page,
};
use crate::model::{FileEntry, Location};

const MAX_SCAN_ENTRIES: usize = 10_000;
const MAX_STAGED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PRINT_PAGES: usize = 10_000;
const PAGE_WIDTH: i32 = 1275; // US Letter at 150 dpi.
const PAGE_HEIGHT: i32 = 1650;
const TEXT_MARGIN: f64 = 75.0;

struct Skipped {
    name: String,
    reason: String,
}

struct Collected {
    paths: Vec<PathBuf>,
    skipped: Vec<Skipped>,
}

struct Prepared {
    _temporary: tempfile::TempDir,
    pages: Vec<PathBuf>,
    files: usize,
    skipped: Vec<Skipped>,
    bytes: u64,
}

impl PreviewState {
    pub(super) fn print_entries(self: &Rc<Self>, entries: Vec<FileEntry>) {
        self.cancel_print();
        self.show_print_progress("Preparing files for printing");
        if let Some(progress) = self.print_progress.borrow().as_ref() {
            progress
                .status
                .set_text(&crate::i18n::tr("Finding files to print…"));
            progress.progress.pulse();
        }
        let state = self.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.batch_print_cancel.replace(Some(cancel.clone()));
        let task = glib::MainContext::default().spawn_local(async move {
            state.prepare_batch(entries, cancel).await;
            state.batch_print_cancel.take();
            state.batch_print_task.take();
        });
        self.batch_print_task.replace(Some(task));
    }

    async fn prepare_batch(self: &Rc<Self>, entries: Vec<FileEntry>, cancel: Arc<AtomicBool>) {
        let parent = self.pane.root().and_downcast::<gtk::Window>();
        let mut skipped = Vec::new();
        let mut roots = Vec::new();
        for entry in entries {
            if let Some(path) = entry.location.native_path() {
                roots.push(path.to_path_buf());
            } else {
                skipped.push(Skipped {
                    name: entry.display_name,
                    reason: crate::i18n::tr("Not a local file"),
                });
            }
        }
        let scan_cancel = cancel.clone();
        let collected = gio::spawn_blocking(move || collect_paths(&roots, &scan_cancel))
            .await
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        skipped.extend(collected.skipped);
        let temporary = match tempfile::tempdir() {
            Ok(temporary) => temporary,
            Err(error) => {
                self.dismiss_print_progress();
                super::show_print_error(
                    parent.as_ref(),
                    &crate::services::io_error_message(&error),
                );
                return;
            }
        };
        let mut prepared = Prepared {
            _temporary: temporary,
            pages: Vec::new(),
            files: 0,
            skipped,
            bytes: 0,
        };
        let total = collected.paths.len();
        for (index, path) in collected.paths.into_iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if prepared.pages.len() >= MAX_PRINT_PAGES {
                prepared.skipped.push(Skipped {
                    name: path.display().to_string(),
                    reason: crate::i18n::tr("Page limit reached"),
                });
                continue;
            }
            if let Some(progress) = self.print_progress.borrow().as_ref() {
                progress.status.set_text(&rust_i18n::t!(
                    "Preparing file %{current} of %{total}",
                    current = crate::i18n::integer((index + 1) as u64),
                    total = crate::i18n::integer(total as u64)
                ));
                progress
                    .progress
                    .set_fraction((index as f64) / (total.max(1) as f64));
            }
            let name = path.display().to_string();
            let entry = match crate::adapters::query_file_entry(Location::local(path)).await {
                Ok(entry) => entry,
                Err(error) => {
                    prepared.skipped.push(Skipped {
                        name,
                        reason: crate::services::gio_error_message(&error),
                    });
                    continue;
                }
            };
            if !crate::ui::browser::entry_supports_printing(&entry) {
                prepared.skipped.push(Skipped {
                    name,
                    reason: crate::i18n::tr("Unsupported file type"),
                });
                continue;
            }
            let file_dir = prepared._temporary.path().join(format!("file-{index}"));
            if let Err(error) = fs::create_dir(&file_dir) {
                prepared.skipped.push(Skipped {
                    name,
                    reason: crate::services::io_error_message(&error),
                });
                continue;
            }
            match self
                .prepare_one_file(
                    &entry,
                    &file_dir,
                    MAX_STAGED_BYTES - prepared.bytes,
                    cancel.clone(),
                )
                .await
            {
                Ok((pages, bytes)) if prepared.pages.len() + pages.len() <= MAX_PRINT_PAGES => {
                    prepared.files += 1;
                    prepared.bytes += bytes;
                    prepared.pages.extend(pages);
                }
                Ok(_) => {
                    let _ = fs::remove_dir_all(&file_dir);
                    prepared.skipped.push(Skipped {
                        name,
                        reason: crate::i18n::tr("Page limit reached"),
                    });
                }
                Err(reason) => {
                    let _ = fs::remove_dir_all(&file_dir);
                    prepared.skipped.push(Skipped { name, reason });
                }
            }
        }
        self.dismiss_print_progress();
        self.show_batch_summary(prepared);
    }

    async fn prepare_one_file(
        self: &Rc<Self>,
        entry: &FileEntry,
        directory: &Path,
        budget: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<(Vec<PathBuf>, u64), String> {
        let mut pages = Vec::new();
        let mut bytes = 0;
        let mut pdf_page = 0;
        loop {
            let content = self.load_batch_page(entry.clone(), pdf_page).await?;
            match content {
                PreviewContent::Text { content, truncated }
                | PreviewContent::Document {
                    source: content,
                    truncated,
                    ..
                } => {
                    if truncated {
                        return Err(crate::i18n::tr(
                            "This text file is too large to print safely.",
                        ));
                    }
                    let directory = directory.to_path_buf();
                    return gio::spawn_blocking(move || {
                        render_text_pages(&content, &directory, budget, &cancel)
                    })
                    .await
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                }
                PreviewContent::Rasterized { png } => {
                    let path = directory.join("page-0.png");
                    bytes += write_page(&path, png, budget - bytes).await?;
                    pages.push(path);
                    return Ok((pages, bytes));
                }
                PreviewContent::Pdf {
                    png,
                    page,
                    pages: count,
                    ..
                } => {
                    if count > MAX_PRINT_PAGES as i32 {
                        return Err(crate::i18n::tr("Page limit reached"));
                    }
                    let count = count.max(1);
                    let path = directory.join(format!("page-{page}.png"));
                    bytes += write_page(&path, png, budget - bytes).await?;
                    pages.push(path);
                    if page.saturating_add(1) >= count {
                        return Ok((pages, bytes));
                    }
                    pdf_page = page.saturating_add(1);
                }
                _ => return Err(crate::i18n::tr("Unsupported file type")),
            }
        }
    }

    async fn load_batch_page(
        self: &Rc<Self>,
        entry: FileEntry,
        pdf_page: i32,
    ) -> Result<PreviewContent, String> {
        let request_id = PreviewRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        let (send, receive) = oneshot::channel();
        let send = Rc::new(RefCell::new(Some(send)));
        let emit = Rc::new(move |event| {
            if matches!(
                event,
                PreviewEvent::Ready(_)
                    | PreviewEvent::Failed { .. }
                    | PreviewEvent::NeedsPassword { .. }
            ) && let Some(send) = send.borrow_mut().take()
            {
                let _ = send.send(event);
            }
        });
        let load = self.provider.load(
            PreviewRequest {
                id: request_id,
                entry,
                text_byte_limit: PRINT_TEXT_BYTE_LIMIT,
                render_document: false,
                pdf_page,
                media_size: self.media_preview_size(),
                model_palette: super::super::theme::ThemeManager::shared().active_model_palette(),
                model_view: Default::default(),
                archive_password: None,
            },
            emit,
        );
        let result = receive
            .await
            .map_err(|_| crate::i18n::tr("Unable to prepare file for printing"));
        drop(load);
        match result? {
            PreviewEvent::Ready(preview) => Ok(preview.content),
            PreviewEvent::Failed { message, .. } => Err(message),
            _ => Err(crate::i18n::tr("Unable to prepare file for printing")),
        }
    }
}

async fn write_page(path: &Path, png: Vec<u8>, budget: u64) -> Result<u64, String> {
    if png.len() as u64 > budget {
        return Err(crate::i18n::tr("Temporary print space limit reached"));
    }
    let path = path.to_path_buf();
    gio::spawn_blocking(move || fs::write(&path, &png).map(|()| png.len() as u64))
        .await
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .map_err(|error| crate::services::io_error_message(&error))
}

fn collect_paths(roots: &[PathBuf], cancel: &AtomicBool) -> Collected {
    let mut paths = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = HashSet::new();
    let mut pending: Vec<_> = roots.iter().rev().cloned().collect();
    let mut scanned = 0;
    while let Some(path) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        scanned += 1;
        if scanned > MAX_SCAN_ENTRIES {
            skipped.push(Skipped {
                name: path.display().to_string(),
                reason: crate::i18n::tr("File scan limit reached"),
            });
            break;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(Skipped {
                    name: path.display().to_string(),
                    reason: crate::services::io_error_message(&error),
                });
                continue;
            }
        };
        if metadata.is_dir() {
            match fs::read_dir(&path) {
                Ok(children) => {
                    let mut found = Vec::new();
                    let remaining = MAX_SCAN_ENTRIES.saturating_sub(scanned);
                    for child in children {
                        if found.len() >= remaining {
                            skipped.push(Skipped {
                                name: path.display().to_string(),
                                reason: crate::i18n::tr("File scan limit reached"),
                            });
                            break;
                        }
                        match child {
                            Ok(child) => found.push(child.path()),
                            Err(error) => skipped.push(Skipped {
                                name: path.display().to_string(),
                                reason: crate::services::io_error_message(&error),
                            }),
                        }
                    }
                    found.sort();
                    pending.extend(found.into_iter().rev());
                }
                Err(error) => skipped.push(Skipped {
                    name: path.display().to_string(),
                    reason: crate::services::io_error_message(&error),
                }),
            }
            continue;
        }
        if metadata.is_symlink() && !fs::metadata(&path).is_ok_and(|target| target.is_file()) {
            skipped.push(Skipped {
                name: path.display().to_string(),
                reason: crate::i18n::tr("Folder links are not followed"),
            });
            continue;
        }
        if !metadata.is_file() && !metadata.is_symlink() {
            continue;
        }
        match fs::canonicalize(&path) {
            Ok(identity) => {
                if seen.insert(identity) {
                    paths.push(path);
                }
            }
            Err(error) => skipped.push(Skipped {
                name: path.display().to_string(),
                reason: crate::services::io_error_message(&error),
            }),
        }
    }
    Collected { paths, skipped }
}

fn render_text_pages(
    text: &str,
    directory: &Path,
    budget: u64,
    cancel: &AtomicBool,
) -> Result<(Vec<PathBuf>, u64), String> {
    let measure = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1)
        .map_err(|error| error.to_string())?;
    let cr = cairo::Context::new(&measure).map_err(|error| error.to_string())?;
    let layout = pangocairo::functions::create_layout(&cr);
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
        "Monospace 15",
    )));
    layout
        .set_width(((PAGE_WIDTH as f64 - 2.0 * TEXT_MARGIN) * f64::from(gtk::pango::SCALE)) as i32);
    layout.set_wrap(gtk::pango::WrapMode::WordChar);
    layout.set_text(text);
    let mut line_ranges = Vec::new();
    let mut iter = layout.iter();
    loop {
        let (top, bottom) = iter.line_yrange();
        line_ranges.push((
            f64::from(top) / f64::from(gtk::pango::SCALE),
            f64::from(bottom) / f64::from(gtk::pango::SCALE),
        ));
        if !iter.next_line() {
            break;
        }
    }
    let starts = super::print_page_starts(&line_ranges, PAGE_HEIGHT as f64 - 2.0 * TEXT_MARGIN);
    if starts.len() > MAX_PRINT_PAGES {
        return Err(crate::i18n::tr("Page limit reached"));
    }
    let mut paths = Vec::new();
    let mut bytes = 0;
    for (index, start) in starts.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, PAGE_WIDTH, PAGE_HEIGHT)
            .map_err(|error| error.to_string())?;
        let cr = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().map_err(|error| error.to_string())?;
        cr.rectangle(
            TEXT_MARGIN,
            TEXT_MARGIN,
            PAGE_WIDTH as f64 - 2.0 * TEXT_MARGIN,
            PAGE_HEIGHT as f64 - 2.0 * TEXT_MARGIN,
        );
        cr.clip();
        cr.translate(TEXT_MARGIN, TEXT_MARGIN - start);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        pangocairo::functions::show_layout(&cr, &layout);
        let path = directory.join(format!("page-{index}.png"));
        let mut file =
            fs::File::create(&path).map_err(|error| crate::services::io_error_message(&error))?;
        surface
            .write_to_png(&mut file)
            .map_err(|error| error.to_string())?;
        file.flush()
            .map_err(|error| crate::services::io_error_message(&error))?;
        bytes += fs::metadata(&path)
            .map_err(|error| crate::services::io_error_message(&error))?
            .len();
        if bytes > budget {
            return Err(crate::i18n::tr("Temporary print space limit reached"));
        }
        paths.push(path);
    }
    Ok((paths, bytes))
}

impl PreviewState {
    fn show_batch_summary(self: &Rc<Self>, prepared: Prepared) {
        let heading = rust_i18n::t!(
            "Ready to print · files: %{files} · pages: %{pages}",
            files = crate::i18n::integer(prepared.files as u64),
            pages = crate::i18n::integer(prepared.pages.len() as u64)
        )
        .into_owned();
        let detail = if prepared.skipped.is_empty() {
            String::new()
        } else {
            rust_i18n::t!(
                "Skipped items: %{count}",
                count = crate::i18n::integer(prepared.skipped.len() as u64)
            )
            .into_owned()
        };
        let can_print = !prepared.pages.is_empty();
        let confirm_label = if can_print {
            crate::i18n::tr("Print")
        } else {
            crate::i18n::tr("Close")
        };
        let Some(overlay) = self
            .pane
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.child())
            .and_downcast::<gtk::Overlay>()
        else {
            return;
        };
        let blurred = overlay
            .child()
            .and_downcast::<super::super::blur::BlurBin>();
        let layout = super::super::controls::modal_layout(
            crate::assets::icons::PRINTER,
            &heading,
            &detail,
            &confirm_label,
        );
        layout.content.add_css_class("compact");
        if !prepared.skipped.is_empty() {
            let skipped = gtk::Label::new(Some(
                &prepared
                    .skipped
                    .iter()
                    .take(20)
                    .map(|item| format!("{} — {}", item.name, item.reason))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
            skipped.set_xalign(0.0);
            skipped.set_wrap(true);
            skipped.set_selectable(true);
            skipped.set_max_width_chars(52);
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_size_request(
                -1,
                (prepared.skipped.len().min(20) as i32 * 36).clamp(48, 180),
            );
            scroll.set_child(Some(&skipped));
            layout.body.append(&scroll);
        }
        layout.cancel.set_visible(can_print);
        let layer = super::super::browser::modal_layer(
            &layout.content,
            &overlay,
            blurred.clone(),
            Some(Rc::new(|| true)),
        );
        overlay.add_overlay(&layer);
        let weak_layer = layer.downgrade();
        let weak_overlay = overlay.downgrade();
        let dismiss = Rc::new(move || {
            if let (Some(layer), Some(overlay)) = (weak_layer.upgrade(), weak_overlay.upgrade()) {
                let blurred = overlay
                    .child()
                    .and_downcast::<super::super::blur::BlurBin>();
                super::super::browser::dismiss_modal_layer(&layer, &overlay, blurred.as_ref());
            }
        });
        let held = Rc::new(RefCell::new(Some(prepared)));
        let close = {
            let held = held.clone();
            let dismiss = dismiss.clone();
            move || {
                held.borrow_mut().take();
                dismiss();
            }
        };
        layout.cancel.connect_clicked({
            let close = close.clone();
            move |_| close()
        });
        layout.close.connect_clicked({
            let close = close.clone();
            move |_| close()
        });
        super::super::modal::close_on_escape(&layer, close);
        layout.confirm.connect_clicked({
            let held = held.clone();
            let dismiss = dismiss.clone();
            let weak_layer = layer.downgrade();
            move |_| {
                let parent = weak_layer
                    .upgrade()
                    .and_then(|layer| layer.root())
                    .and_downcast::<gtk::Window>();
                let prepared = held.borrow_mut().take();
                dismiss();
                if can_print && let Some(prepared) = prepared {
                    print_prepared(prepared, parent.as_ref());
                }
            }
        });
        layout.confirm.grab_focus();
    }
}

fn print_prepared(prepared: Prepared, parent: Option<&gtk::Window>) {
    let operation = prepared_operation(prepared);
    super::run_print_operation(&operation, parent);
}

fn prepared_operation(prepared: Prepared) -> gtk::PrintOperation {
    let operation = gtk::PrintOperation::new();
    operation.set_job_name(&crate::i18n::tr("Print selection"));
    operation.set_n_pages(prepared.pages.len() as i32);
    let pages = prepared.pages.clone();
    operation.connect_draw_page(move |_, context, page| {
        let Some(path) = usize::try_from(page)
            .ok()
            .and_then(|index| pages.get(index))
        else {
            return;
        };
        let Ok(file) = fs::File::open(path) else {
            return;
        };
        let Ok(surface) = cairo::ImageSurface::create_from_png(&mut std::io::BufReader::new(file))
        else {
            return;
        };
        let cr = context.cairo_context();
        let _ = paint_print_page(&cr, &surface, context.width(), context.height());
    });
    let held = Rc::new(RefCell::new(Some(prepared)));
    operation.connect_done(move |_, _| {
        held.borrow_mut().take();
    });
    operation
}

#[cfg(test)]
mod tests;

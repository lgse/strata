// SPDX-License-Identifier: MIT

//! RAW camera metadata footer inside the preview drawer (`raw_details_section` and
//! `load_raw_details` in `src/ui/preview/drawer.rs` and `src/ui/preview/loading.rs`).

use rune::{El, col};
use strata_core::model::FileEntry;
use strata_kit::look::Look;

use crate::{
    App,
    raw_details::{self, RawLoad, RawRows},
    slots::Later,
};

const LABEL_WIDTH: f32 = 128.0;

#[derive(Default)]
pub(super) struct RawSection {
    generation: u64,
    rows: Option<Box<RawRows>>,
    load: Option<RawLoad>,
}

impl RawSection {
    pub(super) fn clear(&mut self) {
        if let Some(mut load) = self.load.take() {
            load.cancel();
        }
        self.rows = None;
    }

    pub(super) fn load(&mut self, entry: &FileEntry, later: Later<App>) {
        self.clear();
        if !raw_details::supports(entry) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.rows = Some(Box::new(raw_details::default_rows()));
        let path = entry.local_thumbnail_path().map(ToOwned::to_owned);
        self.load = Some(raw_details::spawn(path, later, move |app, metadata| {
            let section = &mut app.preview.raw;
            if section.generation != generation || section.rows.is_none() {
                return false;
            }
            section.rows = Some(Box::new(raw_details::rows(&metadata)));
            true
        }));
    }

    pub(super) fn wrap_body(&self, look: &Look, body: El) -> El {
        let Some(rows) = &self.rows else {
            return body;
        };
        let footer = col()
            .key("preview-raw-footer")
            .w_full()
            .px(look.space.lg)
            .py(look.space.md)
            .border_sides([1.0, 0.0, 0.0, 0.0], look.kit.c.border_muted)
            .child(raw_details::section_el(
                look,
                "preview-raw-details",
                rows,
                LABEL_WIDTH,
            ));
        col()
            .key("preview-raw-stack")
            .w_full()
            .h_full()
            .child(col().w_full().grow(1.0).min_h(0.0).child(body))
            .child(footer)
    }
}

// SPDX-License-Identifier: MIT

//! Keyframe cells behind the timeline bubble and seek feedback, decoded in the
//! background after the first frame and cached for the last few clips.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::{
    media::storyboard::Sheet,
    model::FileEntry,
    sandbox::media::{StoryboardEvent, StoryboardSession},
    services::SandboxedMedia,
    ui::preview::audio::details::{LOAD_SETTLE, Lru, TrackKey},
};

pub(super) const CELL_EDGE: u32 = 128;
const STORYBOARD_CACHE: usize = 8;
const POLL: Duration = Duration::from_millis(40);

pub(in crate::ui::preview) struct Storyboard {
    pub(in crate::ui::preview) sheet: Sheet,
    cells: RefCell<Vec<Option<gdk::Texture>>>,
    complete: Cell<bool>,
}

impl Storyboard {
    pub(in crate::ui::preview) fn new(sheet: Sheet) -> Rc<Self> {
        Rc::new(Self {
            sheet,
            cells: RefCell::new(vec![None; sheet.count as usize]),
            complete: Cell::new(false),
        })
    }

    pub(in crate::ui::preview) fn set_cell(&self, index: u32, pixels: Vec<u8>) {
        if index >= self.sheet.count || pixels.len() != self.sheet.cell_bytes() {
            return;
        }
        let texture = gdk::MemoryTexture::new(
            self.sheet.width as i32,
            self.sheet.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            self.sheet.width as usize * 4,
        );
        self.cells.borrow_mut()[index as usize] = Some(texture.upcast::<gdk::Texture>());
    }

    pub(super) fn is_complete(&self) -> bool {
        self.complete.get()
    }

    pub(super) fn loaded_cells(&self) -> usize {
        self.cells.borrow().iter().flatten().count()
    }

    /// The closest decoded cell to `time_us`, so a partial board still answers.
    pub(in crate::ui::preview) fn nearest(&self, time_us: u64) -> Option<gdk::Texture> {
        let cells = self.cells.borrow();
        let wanted = self.sheet.cell_at(time_us) as usize;
        (0..cells.len()).find_map(|distance| {
            wanted
                .checked_sub(distance)
                .and_then(|index| cells[index].clone())
                .or_else(|| cells.get(wanted + distance).cloned().flatten())
        })
    }
}

thread_local! {
    static STORYBOARDS: RefCell<Lru<TrackKey, Rc<Storyboard>>> =
        const { RefCell::new(Lru::new(STORYBOARD_CACHE)) };
}

pub(super) fn cached_storyboard(key: &TrackKey) -> Option<Rc<Storyboard>> {
    STORYBOARDS.with_borrow_mut(|cache| cache.get(key))
}

#[cfg(test)]
pub(super) fn remember_storyboard_for_test(key: TrackKey, storyboard: Rc<Storyboard>) {
    STORYBOARDS.with_borrow_mut(|cache| cache.insert(key, storyboard));
}

type Timer = Rc<RefCell<Option<glib::SourceId>>>;

/// Streams cells into `on_update`; dropping it stops the decode.
pub(super) struct StoryboardLoad(Timer);

impl Drop for StoryboardLoad {
    fn drop(&mut self) {
        if let Some(timer) = self.0.borrow_mut().take() {
            timer.remove();
        }
    }
}

/// A complete cached board answers at once; a partial one is shown and refilled.
pub(super) fn load_storyboard(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_update: impl Fn(Rc<Storyboard>) + 'static,
) -> StoryboardLoad {
    let key = TrackKey::of(entry);
    let cached = cached_storyboard(&key);
    if let Some(board) = &cached {
        on_update(board.clone());
        if board.is_complete() {
            return StoryboardLoad(Timer::default());
        }
    }
    let source = source.clone();
    let session: RefCell<Option<StoryboardSession>> = RefCell::new(None);
    let board: RefCell<Option<Rc<Storyboard>>> = RefCell::new(None);
    let handle = Timer::default();
    let finished = handle.clone();
    let stop = move || {
        finished.borrow_mut().take();
        glib::ControlFlow::Break
    };
    let polling = handle.clone();
    let timer = glib::timeout_add_local_once(LOAD_SETTLE, move || {
        *session.borrow_mut() = StoryboardSession::start(source.clone(), CELL_EDGE);
        let timer = glib::timeout_add_local(POLL, move || {
            let mut session = session.borrow_mut();
            // The waveform or another board holds the slot; wait while this clip is shown.
            if session.is_none() {
                *session = StoryboardSession::start(source.clone(), CELL_EDGE);
                return glib::ControlFlow::Continue;
            }
            while let Some(event) = session.as_ref().and_then(StoryboardSession::receive) {
                match event {
                    StoryboardEvent::Sheet(sheet) => {
                        let fresh = Storyboard::new(sheet);
                        STORYBOARDS
                            .with_borrow_mut(|cache| cache.insert(key.clone(), fresh.clone()));
                        board.replace(Some(fresh));
                    }
                    StoryboardEvent::Cell { index, pixels } => {
                        if let Some(board) = board.borrow().as_ref() {
                            board.set_cell(index, pixels);
                            on_update(board.clone());
                        }
                    }
                    StoryboardEvent::Finished => {
                        if let Some(board) = board.borrow().as_ref() {
                            board.complete.set(true);
                            on_update(board.clone());
                        }
                        return stop();
                    }
                    StoryboardEvent::Failed => return stop(),
                }
            }
            glib::ControlFlow::Continue
        });
        polling.replace(Some(timer));
    });
    handle.replace(Some(timer));
    StoryboardLoad(handle)
}

#[cfg(test)]
mod tests;

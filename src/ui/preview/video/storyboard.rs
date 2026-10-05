// SPDX-License-Identifier: MIT

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

    pub(in crate::ui::preview) fn set_cell(&self, index: u32, pixels: Vec<u8>) -> bool {
        if index >= self.sheet.count || pixels.len() != self.sheet.cell_bytes() {
            return false;
        }
        let mut cells = self.cells.borrow_mut();
        if cells[index as usize].is_some() {
            return false;
        }
        let texture = gdk::MemoryTexture::new(
            self.sheet.width as i32,
            self.sheet.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            self.sheet.width as usize * 4,
        );
        cells[index as usize] = Some(texture.upcast::<gdk::Texture>());
        true
    }

    pub(super) fn is_complete(&self) -> bool {
        self.complete.get()
    }

    #[cfg(test)]
    pub(super) fn loaded_cells(&self) -> usize {
        self.cells.borrow().iter().flatten().count()
    }

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

pub(in crate::ui::preview) fn adopt(key: &TrackKey, sheet: Sheet) -> Rc<Storyboard> {
    STORYBOARDS.with_borrow_mut(|cache| {
        if let Some(board) = cache.get(key).filter(|board| board.sheet == sheet) {
            return board;
        }
        let fresh = Storyboard::new(sheet);
        cache.insert(key.clone(), fresh.clone());
        fresh
    })
}

type Timer = Rc<RefCell<Option<glib::SourceId>>>;

pub(super) struct StoryboardLoad(Timer);

impl Drop for StoryboardLoad {
    fn drop(&mut self) {
        if let Some(timer) = self.0.borrow_mut().take() {
            timer.remove();
        }
    }
}

/// `on_done` lets the waveform claim the shared decode slot. Partial boards
/// retain their cells, but the helper decodes the whole sheet again.
pub(super) fn load_storyboard(
    entry: &FileEntry,
    source: &SandboxedMedia,
    on_update: impl Fn(Rc<Storyboard>) + 'static,
    on_done: impl Fn() + 'static,
) -> StoryboardLoad {
    let on_done = Rc::new(on_done);
    let key = TrackKey::of(entry);
    let cached = cached_storyboard(&key);
    if let Some(board) = &cached {
        on_update(board.clone());
        if board.is_complete() {
            on_done();
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
        on_done();
        glib::ControlFlow::Break
    };
    let polling = handle.clone();
    let timer = glib::timeout_add_local_once(LOAD_SETTLE, move || {
        *session.borrow_mut() = StoryboardSession::start(source.clone(), CELL_EDGE);
        let timer = glib::timeout_add_local(POLL, move || {
            let mut session = session.borrow_mut();
            if session.is_none() {
                *session = StoryboardSession::start(source.clone(), CELL_EDGE);
                return glib::ControlFlow::Continue;
            }
            while let Some(event) = session.as_ref().and_then(StoryboardSession::receive) {
                match event {
                    StoryboardEvent::Sheet(sheet) => {
                        board.replace(Some(adopt(&key, sheet)));
                    }
                    StoryboardEvent::Cell { index, pixels } => {
                        if let Some(board) = board.borrow().as_ref()
                            && board.set_cell(index, pixels)
                        {
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

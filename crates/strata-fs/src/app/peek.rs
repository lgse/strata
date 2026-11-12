// SPDX-License-Identifier: MIT

use crate::{
    app::navigation::LoadState,
    model::{FileEntry, Location},
    services::RequestId,
};

#[derive(Clone, Debug)]
pub struct PeekState {
    pub origin_depth: usize,
    pub location: Location,
    pub entries: Vec<FileEntry>,
    pub load_state: LoadState,
    request_id: RequestId,
}

impl PeekState {
    pub fn new(origin_depth: usize, location: Location, request_id: RequestId) -> Self {
        Self {
            origin_depth,
            location,
            entries: Vec::new(),
            load_state: LoadState::Loading,
            request_id,
        }
    }

    pub fn accepts(&self, request_id: RequestId) -> bool {
        self.request_id == request_id
    }

    pub fn append(&mut self, entries: &[FileEntry]) {
        self.entries.extend_from_slice(entries);
    }

    pub fn finish(&mut self) {
        self.load_state = if self.entries.is_empty() {
            LoadState::Empty
        } else {
            LoadState::Ready
        };
    }

    pub fn fail(&mut self, message: String) {
        self.load_state = LoadState::Error(message);
    }
}

#[cfg(test)]
mod tests;

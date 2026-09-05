// SPDX-License-Identifier: GPL-3.0-or-later

mod file_manager1;
mod gio_location;
mod local_files;
mod local_operations;
mod local_preview;
pub(crate) mod trash;

pub(crate) use file_manager1::{RevealRequest, export_file_manager};
pub(crate) use gio_location::{gio_file_for_location, location_for_file};
pub use local_files::LocalFileSource;
pub use local_operations::LocalOperationProvider;
pub use local_preview::LocalPreviewProvider;

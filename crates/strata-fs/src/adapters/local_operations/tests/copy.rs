// SPDX-License-Identifier: MIT

use rustix::io::Errno;

use super::super::{
    StageContent, StagedSibling,
    copy_engine::{Destination, Options, copy_local},
    publish_staged_without_replace,
};
use super::fs_quirks::{Op, Racer, Support};
use super::*;
use crate::adapters::volume::learn_mount_capability;

mod gvfs;
mod links;
mod names;
mod recursive;
mod refused;
mod remote;
mod specials;
mod staged;

fn make_fifo(path: &Path, mode: u32) -> io::Result<()> {
    rustix::fs::mknodat(
        rustix::fs::CWD,
        path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(mode),
        0,
    )?;
    Ok(())
}

fn paste_into(destination: &Path, items: &[PathBuf]) -> Vec<OperationEvent> {
    run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(destination),
        items: items
            .iter()
            .map(|source| PasteItem {
                source: Location::local(source),
                conflict: TransferConflict::FailIfExists,
                target_name: None,
            })
            .collect(),
        move_sources: false,
    })
}

fn stage(root: &Path, directory: bool) -> Result<StagedSibling, Box<dyn Error>> {
    if directory {
        staged(root, StageContent::Directory)
    } else {
        file_stage(root)
    }
}

fn only_entry(folder: &Path, prefix: &str) -> PathBuf {
    fs::read_dir(folder)
        .expect("list the stage's folder")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        })
        .expect("a stage")
}

/// Each `(path, contents)` beneath `root` reads as its contents.
fn assert_files<C: AsRef<[u8]>>(root: &Path, files: &[(&str, C)]) -> io::Result<()> {
    for (path, contents) in files {
        assert_eq!(fs::read(root.join(path))?, contents.as_ref(), "{path}");
    }
    Ok(())
}

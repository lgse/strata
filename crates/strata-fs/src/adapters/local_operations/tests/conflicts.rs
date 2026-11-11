// SPDX-License-Identifier: MIT

mod decisions;
mod keep_both;
mod noops;
mod prompt;

use super::*;
use crate::{services::ShownName, test_support::MainContextGuard};

/// A fresh folder for one paste, serialized on the main context.
struct Scratch {
    _serial: MainContextGuard<'static>,
    root: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let _serial = serial()?;
        let root = tempfile::tempdir()?;
        Ok(Self { _serial, root })
    }

    fn path(&self) -> &Path {
        self.root.path()
    }
}

fn item(source: impl AsRef<Path>, conflict: TransferConflict) -> PasteItem {
    PasteItem {
        source: Location::local(source.as_ref()),
        conflict,
        target_name: None,
    }
}

fn paste_request(
    id: u64,
    destination: &Path,
    items: Vec<PasteItem>,
    move_sources: bool,
) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(destination),
        items,
        move_sources,
    }
}

/// Runs the paste and collects its events until `is_terminal` sees one.
fn paste_until(
    request: PasteRequest,
    is_terminal: impl Fn(&OperationEvent) -> bool,
) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.paste(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_operation(&events, is_terminal);
    events.take()
}

/// The paste ended, whether it completed, was cancelled, or failed.
fn settled(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::Pasted { .. }
            | OperationEvent::Cancelled { .. }
            | OperationEvent::TransferFailed { .. }
            | OperationEvent::Failed { .. }
    )
}

fn expect_last_pasted(events: &[OperationEvent]) {
    assert!(matches!(events.last(), Some(OperationEvent::Pasted { .. })));
}

/// Each file holds what it should.
fn expect_contents(files: &[(PathBuf, &[u8])]) -> io::Result<()> {
    for (path, contents) in files {
        assert_eq!(fs::read(path)?, *contents, "{}", path.display());
    }
    Ok(())
}

/// Each path exists, or not, as `present` says.
fn expect_present(paths: &[(PathBuf, bool)]) {
    for (path, present) in paths {
        assert_eq!(path.exists(), *present, "{}", path.display());
    }
}

/// Each folder lists exactly the names given.
fn expect_entries(folders: &[(&Path, &[&str])]) -> io::Result<()> {
    for (folder, expected) in folders {
        assert_eq!(names(folder)?, *expected, "{}", folder.display());
    }
    Ok(())
}

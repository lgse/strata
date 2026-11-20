// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::MainContextGuard;

/// A folder whose entry was pasted back into it, as a duplicate is made.
struct Duplicated {
    _serial: MainContextGuard<'static>,
    root: tempfile::TempDir,
    source: PathBuf,
}

impl Duplicated {
    fn destination(&self) -> &Path {
        self.root.path()
    }
}

/// Makes the entry `prepare` writes at the returned path and pastes it into
/// its own folder without a conflict choice.
fn duplicate(
    id: u64,
    prepare: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> Result<Duplicated, Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = prepare(root.path())?;
    run_paste(PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(root.path()),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        move_sources: false,
    });
    Ok(Duplicated {
        _serial,
        root,
        source,
    })
}

fn written(name: impl AsRef<OsStr>, contents: &[u8]) -> impl FnOnce(&Path) -> io::Result<PathBuf> {
    let name = name.as_ref().to_owned();
    move |destination| {
        let source = destination.join(name);
        fs::write(&source, contents)?;
        Ok(source)
    }
}

#[test]
fn duplicating_a_file_preserves_non_utf8_name_bytes() -> Result<(), Box<dyn Error>> {
    let source_name = OsString::from_vec(b"photo-\xff.jpg".to_vec());
    let pasted = duplicate(15, written(source_name, b"original-content"))?;
    let duplicate_name = OsString::from_vec(b"photo-\xff (1).jpg".to_vec());
    let duplicate = pasted.destination().join(duplicate_name);
    assert_eq!(fs::read(duplicate)?, b"original-content");
    Ok(())
}

#[test]
fn duplicating_an_existing_numbered_name_advances_its_index() -> Result<(), Box<dyn Error>> {
    let pasted = duplicate(11, written("photo (1).jpg", b"copy-content"))?;
    assert!(pasted.source.exists());
    assert_eq!(fs::read(&pasted.source)?, b"copy-content");
    let duplicate = pasted.destination().join("photo (2).jpg");
    assert!(duplicate.exists());
    assert_eq!(fs::read(&duplicate)?, b"copy-content");
    Ok(())
}

#[test]
fn duplicating_file_with_existing_numbered_name_advances_to_next_index()
-> Result<(), Box<dyn Error>> {
    let pasted = duplicate(12, |destination| {
        fs::write(destination.join("photo (1).jpg"), b"first copy")?;
        written("photo.jpg", b"original")(destination)
    })?;
    let destination = pasted.destination();
    assert_eq!(fs::read(destination.join("photo (2).jpg"))?, b"original");
    assert_eq!(fs::read(destination.join("photo (1).jpg"))?, b"first copy");
    Ok(())
}

#[test]
fn duplicating_a_directory_generates_numbered_name() -> Result<(), Box<dyn Error>> {
    let pasted = duplicate(13, |destination| {
        let source = destination.join("documents");
        fs::create_dir_all(&source)?;
        fs::write(source.join("notes.txt"), b"nested-file")?;
        Ok(source)
    })?;
    assert!(pasted.source.is_dir());
    let duplicate = pasted.destination().join("documents (1)");
    assert!(duplicate.is_dir());
    assert_eq!(fs::read(duplicate.join("notes.txt"))?, b"nested-file");
    Ok(())
}

#[test]
fn duplicating_hidden_file_generates_correct_numbered_name() -> Result<(), Box<dyn Error>> {
    let pasted = duplicate(30, written(".gitignore", b"target/"))?;
    assert!(pasted.source.exists());
    assert_eq!(
        fs::read(pasted.destination().join(".gitignore (1)"))?,
        b"target/"
    );
    Ok(())
}

#[test]
fn duplicating_multi_extension_file_uses_last_extension_for_candidate_name()
-> Result<(), Box<dyn Error>> {
    let pasted = duplicate(31, written("backup.tar.gz", b"contents"))?;
    assert!(pasted.source.exists());
    assert_eq!(
        fs::read(pasted.destination().join("backup.tar (1).gz"))?,
        b"contents"
    );
    Ok(())
}

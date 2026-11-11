// SPDX-License-Identifier: MIT

use strata_components::{
    contract::{EntryKind, LinkTarget},
    rename::{Initial, initial_selection},
};
use strata_fs::services::rename_stem_end;

/// The editor selects by its own rule (`strata_components::rename::initial_selection`), in
/// bytes; the GTK field selects up to `rename_stem_end`, in characters, and the whole of a
/// folder's name. They are one rule on the names gate G3 renames and on the kinds of name
/// that tell rules apart: a leading dot, several dots, a trailing dot, none, and characters
/// of more than one byte.
#[test]
fn the_editors_first_selection_is_stratas_stem_rule() {
    for name in [
        "todo.txt",
        "readme.md",
        "archive",
        "synthetic-quarterly-report-with-a-very-long-descriptive-basename-2026.txt",
        "archive.tar.gz",
        ".bashrc",
        ".config.toml",
        "..hidden.txt",
        "README",
        "trailing.",
        "a.b.c",
        "résumé.pdf",
        "日本語.txt",
        "māo",
    ] {
        let (start, end) = initial_selection(name, EntryKind::File, Initial::Stem);
        let characters = name[..end].chars().count();
        let stem = usize::try_from(rename_stem_end(name)).expect("an offset");
        assert_eq!((start, characters), (0, stem), "{name}");

        for folder in [EntryKind::Folder, EntryKind::Symlink(LinkTarget::Folder)] {
            let whole = initial_selection(name, folder, Initial::Stem);
            assert_eq!(whole, (0, name.len()), "{name} as {folder:?}");
        }
    }
}

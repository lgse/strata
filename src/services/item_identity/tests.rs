// SPDX-License-Identifier: MIT

use rustix::fs::FileType;

use super::{Attributes, Evidence, IdentityTraits, InodeNumbers, ItemIdentity};

const KEPT: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Kept,
    folder_births: true,
};
const RENEWED: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Renewed,
    folder_births: true,
};
const EXFAT: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Renewed,
    folder_births: false,
};
const UNKNOWN: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Unknown,
    folder_births: true,
};

const FOLDER: Evidence = Evidence {
    device: 7,
    inode: 10,
    kind: FileType::Directory,
    birth: Some((100, 0)),
    attributes: None,
};
const FILE: Evidence = Evidence {
    kind: FileType::RegularFile,
    attributes: Some(Attributes {
        size: 5,
        mtime: (90, 0),
    }),
    ..FOLDER
};

fn renumbered(evidence: Evidence) -> Evidence {
    Evidence {
        inode: 11,
        ..evidence
    }
}

fn reborn(evidence: Evidence) -> Evidence {
    Evidence {
        birth: Some((200, 0)),
        ..evidence
    }
}

fn without_birth(evidence: Evidence) -> Evidence {
    Evidence {
        birth: None,
        ..evidence
    }
}

fn zero_birth(evidence: Evidence) -> Evidence {
    Evidence {
        birth: Some((0, 0)),
        ..evidence
    }
}

#[test]
fn an_item_is_known_again_only_by_what_its_mount_proves() {
    let rewritten = Evidence {
        attributes: Some(Attributes {
            size: 6,
            mtime: (95, 0),
        }),
        ..without_birth(renumbered(FILE))
    };
    let moved_away = Evidence {
        device: 8,
        ..FOLDER
    };
    let another_kind = Evidence {
        kind: FileType::Symlink,
        ..FILE
    };
    let folder_with_attributes = Evidence {
        attributes: FILE.attributes,
        ..without_birth(FOLDER)
    };
    let rows = [
        (
            "renamed where numbers are kept",
            KEPT,
            FOLDER,
            FOLDER,
            KEPT,
            true,
        ),
        (
            "number reused by a new item",
            KEPT,
            FOLDER,
            reborn(FOLDER),
            KEPT,
            false,
        ),
        (
            "another number where numbers are kept",
            KEPT,
            FOLDER,
            renumbered(FOLDER),
            KEPT,
            false,
        ),
        (
            "renumbered, birth kept",
            RENEWED,
            FOLDER,
            renumbered(FOLDER),
            RENEWED,
            true,
        ),
        (
            "renumbered, born again",
            RENEWED,
            FOLDER,
            reborn(renumbered(FOLDER)),
            RENEWED,
            false,
        ),
        (
            "renumbered file, no births, same size and time",
            RENEWED,
            without_birth(FILE),
            without_birth(renumbered(FILE)),
            RENEWED,
            true,
        ),
        (
            "renumbered file, no births, rewritten",
            RENEWED,
            without_birth(FILE),
            rewritten,
            RENEWED,
            false,
        ),
        (
            "renumbered folder, no births",
            RENEWED,
            without_birth(FOLDER),
            without_birth(renumbered(FOLDER)),
            RENEWED,
            false,
        ),
        (
            "renumbered folder, no births, same size and time",
            RENEWED,
            folder_with_attributes,
            renumbered(folder_with_attributes),
            RENEWED,
            false,
        ),
        (
            "renumbered folder, zero births",
            RENEWED,
            zero_birth(FOLDER),
            zero_birth(renumbered(FOLDER)),
            RENEWED,
            false,
        ),
        (
            "exFAT folder, birth kept",
            EXFAT,
            FOLDER,
            renumbered(FOLDER),
            EXFAT,
            false,
        ),
        (
            "exFAT file, birth kept",
            EXFAT,
            FILE,
            renumbered(FILE),
            EXFAT,
            true,
        ),
        (
            "unknown mount, same number and birth",
            UNKNOWN,
            FOLDER,
            FOLDER,
            UNKNOWN,
            true,
        ),
        (
            "unknown mount, renumbered",
            UNKNOWN,
            FOLDER,
            renumbered(FOLDER),
            UNKNOWN,
            false,
        ),
        (
            "unknown mount, same number, no births",
            UNKNOWN,
            without_birth(FOLDER),
            without_birth(FOLDER),
            UNKNOWN,
            true,
        ),
        (
            "read where numbers were kept, now renewed",
            KEPT,
            FOLDER,
            renumbered(FOLDER),
            RENEWED,
            true,
        ),
        ("another device", KEPT, FOLDER, moved_away, KEPT, false),
        ("another kind", RENEWED, FILE, another_kind, RENEWED, false),
    ];
    for (row, recorded_on, recorded, current, current_on, same) in rows {
        let recorded = ItemIdentity {
            evidence: recorded,
            traits: recorded_on,
        };
        let current = ItemIdentity {
            evidence: current,
            traits: current_on,
        };
        assert_eq!(recorded.same(&current), same, "{row}");
        assert_eq!(
            current.same(&recorded),
            same,
            "{row}, compared the other way"
        );
    }
}

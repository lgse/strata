// SPDX-License-Identifier: MIT

use rustix::fs::FileType;

use super::{Attributes, Evidence, IdentityTraits, InodeNumbers, ItemIdentity};

const KEPT: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Kept,
    folder_births: true,
    made_up_births: false,
};
const RENEWED: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Renewed,
    ..KEPT
};
const EXFAT: IdentityTraits = IdentityTraits {
    folder_births: false,
    ..RENEWED
};
const UNKNOWN: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Unknown,
    ..KEPT
};
const SMB: IdentityTraits = IdentityTraits {
    made_up_births: true,
    ..RENEWED
};
const SMB_SERVER_NUMBERS: IdentityTraits = IdentityTraits {
    numbers: InodeNumbers::Kept,
    ..SMB
};

const FOLDER: Evidence = Evidence {
    device: 7,
    inode: 10,
    kind: FileType::Directory,
    birth: Some((100, 0)),
    handle: None,
    attributes: None,
};
const FILE: Evidence = Evidence {
    kind: FileType::RegularFile,
    attributes: Some(Attributes {
        size: 5,
        mtime: (90, 0),
        ctime: Some((95, 0)),
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

fn changed(evidence: Evidence) -> Evidence {
    let attributes = evidence.attributes.map(|attributes| Attributes {
        ctime: Some((120, 0)),
        ..attributes
    });
    Evidence {
        attributes,
        ..evidence
    }
}

fn handled(evidence: Evidence, handle: u64) -> Evidence {
    Evidence {
        handle: Some(handle),
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
            ctime: Some((95, 0)),
        }),
        ..without_birth(renumbered(FILE))
    };
    let resized = Evidence {
        attributes: FILE.attributes.map(|attributes| Attributes {
            size: 6,
            ..attributes
        }),
        ..renumbered(FILE)
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
            true,
        ),
        (
            "number reused by a new item",
            KEPT,
            FOLDER,
            reborn(FOLDER),
            KEPT,
            false,
            false,
        ),
        (
            "another number where numbers are kept",
            KEPT,
            FOLDER,
            renumbered(FOLDER),
            KEPT,
            false,
            false,
        ),
        (
            "renumbered, birth kept",
            RENEWED,
            FOLDER,
            renumbered(FOLDER),
            RENEWED,
            true,
            true,
        ),
        (
            "renumbered, born again",
            RENEWED,
            FOLDER,
            reborn(renumbered(FOLDER)),
            RENEWED,
            false,
            false,
        ),
        (
            "renumbered file, no births, same size and time",
            RENEWED,
            without_birth(FILE),
            without_birth(renumbered(FILE)),
            RENEWED,
            true,
            true,
        ),
        (
            "renumbered file, no births, rewritten",
            RENEWED,
            without_birth(FILE),
            rewritten,
            RENEWED,
            false,
            false,
        ),
        (
            "renumbered folder, no births",
            RENEWED,
            without_birth(FOLDER),
            without_birth(renumbered(FOLDER)),
            RENEWED,
            false,
            false,
        ),
        (
            "renumbered folder, no births, same size and time",
            RENEWED,
            folder_with_attributes,
            renumbered(folder_with_attributes),
            RENEWED,
            false,
            false,
        ),
        (
            "renumbered folder, zero births",
            RENEWED,
            zero_birth(FOLDER),
            zero_birth(renumbered(FOLDER)),
            RENEWED,
            false,
            false,
        ),
        (
            "folder, no births, number still the same",
            RENEWED,
            without_birth(FOLDER),
            without_birth(FOLDER),
            RENEWED,
            false,
            true,
        ),
        (
            "exFAT folder whose birth a rename reset, number still the same",
            EXFAT,
            FOLDER,
            reborn(FOLDER),
            EXFAT,
            false,
            false,
        ),
        (
            "exFAT folder, birth kept",
            EXFAT,
            FOLDER,
            renumbered(FOLDER),
            EXFAT,
            false,
            false,
        ),
        (
            "exFAT file, birth kept",
            EXFAT,
            FILE,
            renumbered(FILE),
            EXFAT,
            true,
            true,
        ),
        (
            "exFAT file born in the same tick, another size",
            EXFAT,
            FILE,
            resized,
            EXFAT,
            false,
            false,
        ),
        (
            "exFAT file born in the same tick, recorded without size and time",
            EXFAT,
            Evidence {
                attributes: None,
                ..FILE
            },
            resized,
            EXFAT,
            false,
            false,
        ),
        (
            "unknown mount, renumbered",
            UNKNOWN,
            FOLDER,
            renumbered(FOLDER),
            UNKNOWN,
            false,
            false,
        ),
        (
            "unknown mount, same number, no births",
            UNKNOWN,
            without_birth(FOLDER),
            without_birth(FOLDER),
            UNKNOWN,
            true,
            true,
        ),
        (
            "read where numbers were kept, now renewed",
            KEPT,
            FOLDER,
            renumbered(FOLDER),
            RENEWED,
            true,
            true,
        ),
        (
            "change time moved where births are real",
            RENEWED,
            FILE,
            changed(renumbered(FILE)),
            RENEWED,
            true,
            true,
        ),
        (
            "SMB file, birth and attributes kept",
            SMB,
            FILE,
            renumbered(FILE),
            SMB,
            true,
            true,
        ),
        (
            "SMB file whose cached number and birth outlived a replacement",
            SMB,
            FILE,
            changed(FILE),
            SMB,
            false,
            false,
        ),
        (
            "SMB file, no births, attributes kept",
            SMB,
            without_birth(FILE),
            without_birth(renumbered(FILE)),
            SMB,
            true,
            true,
        ),
        (
            "SMB folder, birth kept",
            SMB,
            FOLDER,
            renumbered(FOLDER),
            SMB,
            true,
            true,
        ),
        (
            "SMB file, another server number and a repeated birth",
            SMB_SERVER_NUMBERS,
            FILE,
            renumbered(FILE),
            SMB_SERVER_NUMBERS,
            false,
            false,
        ),
        (
            "SMB file read with server numbers, decided after they went",
            SMB_SERVER_NUMBERS,
            FILE,
            changed(renumbered(FILE)),
            SMB,
            false,
            false,
        ),
        (
            "number reused without births, another file handle",
            KEPT,
            handled(without_birth(FILE), 1),
            handled(without_birth(FILE), 2),
            KEPT,
            false,
            false,
        ),
        (
            "same number and file handle without births",
            KEPT,
            handled(without_birth(FILE), 1),
            handled(without_birth(FILE), 1),
            KEPT,
            true,
            true,
        ),
        (
            "a file handle on one side only",
            KEPT,
            handled(without_birth(FILE), 1),
            without_birth(FILE),
            KEPT,
            true,
            true,
        ),
        (
            "folder, no births, another file handle",
            RENEWED,
            handled(without_birth(FOLDER), 1),
            handled(without_birth(FOLDER), 2),
            RENEWED,
            false,
            false,
        ),
        (
            "another device",
            KEPT,
            FOLDER,
            moved_away,
            KEPT,
            false,
            false,
        ),
        (
            "another kind",
            RENEWED,
            FILE,
            another_kind,
            RENEWED,
            false,
            false,
        ),
    ];
    for (row, recorded_on, recorded, current, current_on, same, unchanged) in rows {
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
        assert_eq!(recorded.unchanged(&current), unchanged, "{row}, unchanged");
    }
}

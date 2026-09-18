// SPDX-License-Identifier: MIT

use std::io::Cursor;

use super::{
    RootEntry, desktop_icon, dir_icon_candidates, embedded_image_offset, icon_name_candidates,
    root_entries,
};

fn elf64(sections: &[(u32, u64, u64)], tail: &[u8]) -> Vec<u8> {
    let shoff = 0x40u64;
    let mut image = vec![0u8; shoff as usize];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
    image[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
    image[0x3c..0x3e].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    for &(kind, offset, size) in sections {
        let mut section = [0u8; 64];
        section[4..8].copy_from_slice(&kind.to_le_bytes());
        section[24..32].copy_from_slice(&offset.to_le_bytes());
        section[32..40].copy_from_slice(&size.to_le_bytes());
        image.extend_from_slice(&section);
    }
    for &(kind, offset, size) in sections {
        if kind == 8 {
            continue;
        }
        let required = (offset + size) as usize;
        if required > image.len() {
            image.resize(required, 0);
        }
    }
    image.extend_from_slice(tail);
    image
}

fn entry(name: &str, is_file: bool, symlink_target: Option<&str>) -> RootEntry {
    RootEntry {
        name: name.to_owned(),
        is_file,
        symlink_target: symlink_target.map(str::to_owned),
    }
}

#[test]
fn elf_offset_stops_at_last_allocated_section_and_requires_squashfs_magic() {
    let image = elf64(
        &[(1, 0x40, 0x200), (8, 0x9000, 0x8000), (1, 0x240, 0x40)],
        b"hsqsrest",
    );
    assert_eq!(embedded_image_offset(&mut Cursor::new(image)), Some(0x280));

    let unsigned = elf64(&[(1, 0x40, 0x200)], b"nope");
    assert_eq!(embedded_image_offset(&mut Cursor::new(unsigned)), None);
}

#[test]
fn elf_offset_rejects_non_elf_and_truncated_images() {
    assert_eq!(
        embedded_image_offset(&mut Cursor::new(b"#!/bin/sh".to_vec())),
        None
    );
    assert_eq!(embedded_image_offset(&mut Cursor::new(Vec::new())), None);

    let mut header_only = elf64(&[(1, 0x40, 0x200)], b"hsqs");
    header_only.truncate(0x40);
    assert_eq!(embedded_image_offset(&mut Cursor::new(header_only)), None);
}

#[test]
fn elf_offset_reads_32_bit_and_big_endian_headers() {
    let shoff = 0x34u32;
    let mut image = vec![0u8; shoff as usize + 40 + 4];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 1;
    image[5] = 2;
    image[0x20..0x24].copy_from_slice(&shoff.to_be_bytes());
    image[0x2e..0x30].copy_from_slice(&40u16.to_be_bytes());
    image[0x30..0x32].copy_from_slice(&1u16.to_be_bytes());
    image[shoff as usize + 4..shoff as usize + 8].copy_from_slice(&1u32.to_be_bytes());
    image[shoff as usize + 16..shoff as usize + 20].copy_from_slice(&0x80u32.to_be_bytes());
    image[shoff as usize + 20..shoff as usize + 24].copy_from_slice(&0x10u32.to_be_bytes());
    let offset = (0x80u32 + 0x10u32) as usize;
    image.resize(offset, 0);
    image.extend_from_slice(b"hsqs");
    assert_eq!(embedded_image_offset(&mut Cursor::new(image)), Some(0x90));
}

#[test]
fn root_entries_keeps_top_level_members_and_symlink_targets() {
    let listing = "Parallel unsquashfs: Using 8 processors\n\
        drwxr-xr-x root/root 60 2026-06-28 12:07 squashfs-root\n\
        lrwxrwxrwx root/root 118 2026-06-28 12:05 squashfs-root/.DirIcon -> /build/KamehaDB.png\n\
        -rw-r--r-- root/root 65074 2026-06-28 12:05 squashfs-root/KamehaDB.png\n\
        -rw-r--r-- root/root 274 2026-06-28 12:05 squashfs-root/KamehaDB.desktop\n\
        -rw-r--r-- root/root 99 2026-06-28 12:05 squashfs-root/usr/share/icons/deep.png\n";
    let entries = root_entries(listing);
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[0].symlink_target.as_deref(),
        Some("/build/KamehaDB.png")
    );
    assert!(!entries[0].is_file);
    assert!(entries[1].is_file);
}

#[test]
fn dir_icon_prefers_regular_file_then_resolves_broken_build_host_symlinks() {
    let regular = dir_icon_candidates(&[entry(".DirIcon", true, None)]);
    assert_eq!(regular, vec![".DirIcon".to_owned()]);

    let resolved = dir_icon_candidates(&[
        entry(
            ".DirIcon",
            false,
            Some("/home/runner/work/x/AppDir/KamehaDB.png"),
        ),
        entry("KamehaDB.png", true, None),
    ]);
    assert_eq!(resolved, vec!["KamehaDB.png".to_owned()]);

    let relative =
        dir_icon_candidates(&[entry(".DirIcon", false, Some("usr/share/icons/app.png"))]);
    assert_eq!(relative, vec!["usr/share/icons/app.png".to_owned()]);

    let missing = dir_icon_candidates(&[entry(".DirIcon", false, Some("/build/gone.png"))]);
    assert!(missing.is_empty());
    assert!(dir_icon_candidates(&[]).is_empty());
}

#[test]
fn desktop_icon_name_matches_root_files_case_insensitively() {
    let desktop = b"[Desktop Entry]\nName=KamehaDB\nIcon=kamehadb\nExec=app\n";
    assert_eq!(desktop_icon(desktop).as_deref(), Some("kamehadb"));
    assert_eq!(desktop_icon(b"[Desktop Entry]\nName=X\n"), None);

    let entries = [
        entry("KamehaDB.png", true, None),
        entry("kamehadb.txt", true, None),
        entry("other.png", true, None),
    ];
    assert_eq!(
        icon_name_candidates(&entries, "kamehadb"),
        vec!["KamehaDB.png".to_owned()]
    );
    assert_eq!(
        icon_name_candidates(&entries, "KamehaDB.PNG"),
        vec!["KamehaDB.png".to_owned()]
    );
    assert!(icon_name_candidates(&entries, "absent").is_empty());
    assert_eq!(
        icon_name_candidates(&entries, "icons/app.png"),
        vec!["icons/app.png".to_owned()]
    );
}

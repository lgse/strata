// SPDX-License-Identifier: MIT

mod duplicates;
mod fixing;
mod lab;
mod numbering;
mod refusals;

use super::super::names::fix::fix_name;
use super::*;
use crate::{
    adapters::volume::{MountType, NameEncoding, NameProfile, PrivateUse, Reserved, gvfs_profile},
    services::RenameCause,
};

fn fat() -> NameProfile {
    MountType {
        fs_type: "vfat".to_owned(),
        super_options: "rw,utf8".to_owned(),
    }
    .name_profile(None, 1530)
}

fn sfm_smb() -> NameProfile {
    NameProfile {
        mapped: Some(PrivateUse::Sfm),
        ..NameProfile::smb(Some(255))
    }
}

/// What fixing `name` for `profile` gives, and why it changed.
fn fixed(name: &OsStr, profile: &NameProfile, folder: bool) -> (OsString, Option<RenameCause>) {
    let fixed = fix_name(name, profile, folder);
    (fixed.name.into_owned(), fixed.cause)
}

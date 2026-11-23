// SPDX-License-Identifier: MIT

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub(super) fn icd() -> Option<OsString> {
    if let Some(icd) = std::env::var_os("STRATA_VK_ICD").filter(|v| !v.is_empty()) {
        return Some(icd);
    }
    let dir = Path::new("/usr/share/vulkan/icd.d");
    let preferred = [
        format!("lvp_icd.{}.json", std::env::consts::ARCH),
        "lvp_icd.json".to_owned(),
    ];
    if let Some(icd) = preferred.iter().map(|n| dir.join(n)).find(|p| p.is_file()) {
        return Some(icd.into_os_string());
    }
    let mut lavapipe: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("lvp_icd") && n.ends_with(".json"))
        })
        .collect();
    lavapipe.sort();
    std::env::join_paths(lavapipe)
        .ok()
        .filter(|joined| !joined.is_empty())
}

/// `STRATA_WLR_RENDERER`, else pixman when every ICD in `icd` is lavapipe, else gles2.
pub(super) fn renderer(icd: Option<&OsStr>) -> String {
    if let Some(r) = std::env::var("STRATA_WLR_RENDERER")
        .ok()
        .filter(|r| !r.is_empty())
    {
        return r;
    }
    let lavapipe = icd.is_some_and(|icd| {
        std::env::split_paths(icd)
            .all(|p| std::fs::read_to_string(&p).is_ok_and(|json| json.contains("libvulkan_lvp")))
    });
    if lavapipe { "pixman" } else { "gles2" }.to_owned()
}

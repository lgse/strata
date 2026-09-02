// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FileEntry, MetadataValue};

pub fn modified_date(entry: &FileEntry) -> String {
    let MetadataValue::Known(seconds) = entry.modified_unix_seconds else {
        return "—".to_owned();
    };
    let Some(modified) = glib::DateTime::from_unix_local(seconds).ok() else {
        return "—".to_owned();
    };
    let Some(now) = glib::DateTime::now_local().ok() else {
        return modified
            .format("%Y-%m-%d %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned());
    };

    let span = now.difference(&modified).0;
    let day_diff = span / 86_400_000_000;
    let same_year = now.year() == modified.year();

    if day_diff == 0 {
        let hours = span / 3_600_000_000;
        let minutes = span / 60_000_000;
        if hours >= 1 {
            format!("{}h ago", hours)
        } else if minutes >= 1 {
            format!("{}m ago", minutes)
        } else {
            "just now".to_owned()
        }
    } else if day_diff == 1 {
        modified
            .format("%H:%M")
            .map(|s| format!("Yesterday, {}", s))
            .unwrap_or_else(|_| "—".to_owned())
    } else if day_diff < 7 {
        modified
            .format("%A %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    } else if same_year {
        modified
            .format("%b %-d, %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    } else {
        modified
            .format("%b %-d, %Y")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    }
}

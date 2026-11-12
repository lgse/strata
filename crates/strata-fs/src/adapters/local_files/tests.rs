// SPDX-License-Identifier: MIT

mod browse;
mod enumeration;
mod fences;
mod icon_details;
mod live_monitor;
mod logging;
mod metadata_fill;
mod monitor_queue;
mod recent_listing;
mod recent_removal;
mod stalled_mounts;
mod stamps;
mod support;
mod trash;
mod validation;

use std::{
    error::Error,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
};

use super::*;
use crate::test_support::{ASYNC_MAIN_CONTEXT_DEFAULT, Gate, capture_logs};
use support::*;

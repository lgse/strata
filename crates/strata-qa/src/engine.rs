// SPDX-License-Identifier: MIT

//! What the QA mode decides without knowing the toolkit: which nodes a snapshot reports, in
//! what order, and what each of them is.

mod fields;
mod lookup;
mod scan;

pub use self::{
    lookup::{descendants, presented, rendered},
    scan::{PaneNodes, Scene, scan},
};

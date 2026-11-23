// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

/// Drag-and-drop binder and drop-target contracts.
pub mod dnd;
/// Entry metadata traits shared across collection views.
pub mod entry;
/// Shared geometry helpers.
pub mod geometry;
/// Filter-match highlight spans.
pub mod matched;

pub use dnd::{
    ActionInput, CrossDevice, Dnd, DragPayload, DragSource, DropAction, DropActions, DropData,
    DropHover, DropItem, DropOffer, DropZone, Dropped, ElementKey, ReadFailure, URI_LIST,
    VolumeRelation, choose_action,
};
pub use entry::{
    Entry, EntryGlyph, EntryId, EntryKind, EntrySource, LinkTarget, ListEntry, ListSource,
    OwnedGlyph, ThumbKey,
};
pub use geometry::{CollectionGeometry, Direction, IndexRanges, Point, Rect, UniformRows};
pub use matched::{FuzzyMatch, MatchedText, char_ranges, fuzzy, normalize_ranges};

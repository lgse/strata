//! Shared component contracts: entry ids and sources, collection geometry, drag and drop
//! intents and matched-text labels.
//!
//! These types let the collection, chrome and drag-and-drop work build at the same time
//! without waiting on each other. Each exists because two or more components need it, and
//! each type's docs end with the work packages that use it. They grow by addition: a change
//! that removes or renames something is its own decision.
//!
//! - [`entry`]: [`EntryId`], [`EntryKind`], [`Entry`] and the index-based [`EntrySource`].
//! - [`geometry`]: [`CollectionGeometry`], which marquee, keyboard moves and context-menu
//!   anchors go through, and its uniform-row implementation [`UniformRows`].
//! - [`dnd`]: drag payloads, drop offers and zones, the copy-or-move rule
//!   ([`choose_action`]) and the [`Dnd`] binder that attaches them to elements.
//! - [`matched`]: [`MatchedText`], a label that draws matched ranges with a Kit accent.
//!
//! Work package: W1, `docs/work/phase-0-groundwork/W1-component-contracts.md`.

#![warn(missing_docs)]

pub mod dnd;
pub mod entry;
pub mod geometry;
pub mod matched;

pub use dnd::{
    ActionInput, CrossDevice, Dnd, DragPayload, DragSource, DropAction, DropActions, DropData,
    DropHover, DropItem, DropOffer, DropZone, Dropped, ElementKey, URI_LIST, VolumeRelation,
    choose_action,
};
pub use entry::{
    Entry, EntryId, EntryKind, EntrySource, LinkTarget, ListEntry, ListSource, ThumbKey,
};
pub use geometry::{CollectionGeometry, Direction, IndexRanges, Point, Rect, UniformRows};
pub use matched::{FuzzyMatch, MatchedText, char_ranges, fuzzy, normalize_ranges};

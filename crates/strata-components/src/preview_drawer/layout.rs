//! The width rules: where the drawer's slot ends up in the split, solved from the split's
//! width, the view beside it and the manual width. `docs/preview-panel-layout.md` in Strata
//! (at the plan's watermark) is the source of every number here.

/// The drawer's width rules, in points. [`DrawerRules::STRATA`] is Strata's set; a host may
/// pass its own (the chooser uses the same set).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DrawerRules {
    /// The preview's content hides when less than this would remain beside the focused
    /// column (Strata's `MIN_SPLIT_PREVIEW_WIDTH`). It is also the narrowest the content box
    /// is ever laid out at.
    pub hide_below: f32,
    /// The least manual width: one standard column. In Columns the larger of this and
    /// [`ColumnsFit::standard_column`] is the floor.
    pub manual_min: f32,
    /// The least automatic width in Icons and List (Strata's `MIN_WIDTH`).
    pub auto_min: f32,
    /// The widest automatic width in Icons and List (Strata's `MAX_WIDTH`).
    pub auto_max: f32,
    /// The automatic width's share of the split in Icons and List: 90% of half.
    pub auto_share: f32,
    /// What Icons and List keep for the files beside the preview: one standard column.
    pub files_min: f32,
    /// One standard column: the Columns minimum, unless C2 reports another
    /// ([`ColumnsFit::standard_column`]).
    pub standard_column: f32,
    /// The divider's line.
    pub line: f32,
    /// The grip inside the preview's leading edge that moves the divider.
    pub grip: f32,
    /// One keyboard or assistive step of the divider. Page Up and Page Down move ten. An
    /// assumption: Strata leaves the step to GtkPaned, and this hasn't been measured against
    /// it.
    pub step: f32,
}

impl DrawerRules {
    /// Strata's rules.
    pub const STRATA: DrawerRules = DrawerRules {
        hide_below: 240.0,
        manual_min: 300.0,
        auto_min: 240.0,
        auto_max: 3000.0,
        auto_share: 0.45,
        files_min: 300.0,
        standard_column: 300.0,
        line: 1.0,
        grip: 6.0,
        step: 8.0,
    };
}

impl Default for DrawerRules {
    fn default() -> Self {
        Self::STRATA
    }
}

/// The view beside the drawer, which decides its automatic width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arrangement {
    /// C2 ColumnsView: the drawer fills the space right of the navigated columns.
    Columns(ColumnsFit),
    /// C3 IconsGrid.
    Icons,
    /// C4 ListTable.
    List,
}

/// What C2 reports about its strip each frame, in points. Grows by addition: build it with
/// [`ColumnsFit::new`] and the setters.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColumnsFit {
    /// The columns from the root through the focused one, as the strip shows them.
    pub navigated: f32,
    /// The least width of the files side that keeps the focused column fully visible.
    pub keep_focused: f32,
    /// [`Self::keep_focused`] with the focused column counted at most one standard column:
    /// what the narrow-window test leaves beside the preview.
    pub keep_focused_standard: f32,
    /// Trailing columns the empty slot lends its width to (a focused folder's child column).
    pub trailing: f32,
    /// One standard column: the Columns minimum.
    pub standard_column: f32,
}

impl ColumnsFit {
    /// A strip whose navigated columns take `navigated` and whose focused column stays
    /// visible in `keep_focused`. No trailing columns; the rules' standard column.
    pub fn new(rules: &DrawerRules, navigated: f32, keep_focused: f32) -> Self {
        ColumnsFit {
            navigated,
            keep_focused,
            keep_focused_standard: keep_focused,
            trailing: 0.0,
            standard_column: rules.standard_column,
        }
    }
    pub fn keep_focused_standard(mut self, w: f32) -> Self {
        self.keep_focused_standard = w;
        self
    }
    pub fn trailing(mut self, w: f32) -> Self {
        self.trailing = w;
        self
    }
    pub fn standard_column(mut self, w: f32) -> Self {
        self.standard_column = w;
        self
    }
}

/// The solved widths, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Solved {
    /// The preview's width while it shows: the automatic or manual width, within its range.
    pub width: f32,
    /// The width the content box is laid out at: [`Self::width`], never below
    /// [`DrawerRules::hide_below`]. The slot clips it.
    pub pane: f32,
    /// The empty reserved slot's width in Columns: the width and the divider's line, as the
    /// slot takes with the preview, less the trailing columns it lends, down to 0.
    pub empty_slot: f32,
    /// Whether the content fits beside the focused column. False is the narrow-window rule:
    /// the content hides, and the host pauses media and defers loading.
    pub fits: bool,
    /// The divider's range: the least manual width and the widest the files allow.
    pub min: f32,
    pub max: f32,
}

/// The widths for a split `available` points wide, beside `arrangement`, with an optional
/// manual width. Pure, so C2 and the host can ask the same question the drawer does.
pub fn solve(
    rules: &DrawerRules,
    arrangement: &Arrangement,
    available: f32,
    manual: Option<f32>,
) -> Solved {
    let room = (available - rules.line).max(0.0);
    let (occupied, keep, keep_standard, trailing, columns_min) = match arrangement {
        Arrangement::Columns(fit) => (
            fit.navigated,
            fit.keep_focused,
            fit.keep_focused_standard,
            fit.trailing,
            Some(fit.standard_column),
        ),
        Arrangement::Icons | Arrangement::List => (
            room - room * rules.auto_share,
            rules.files_min,
            rules.files_min,
            0.0,
            None,
        ),
    };
    let max = (room - keep).max(0.0);
    // The least manual width: one standard column. In Columns, C2's standard column counts
    // too, and the larger of the two wins.
    let manual_floor =
        columns_min.map_or(rules.manual_min, |standard| standard.max(rules.manual_min));
    let floor = match (manual, columns_min) {
        (Some(_), _) => manual_floor,
        (None, Some(standard)) => standard,
        (None, None) => rules.auto_min,
    };
    let min = floor.min(max);
    let free = (room - occupied).max(0.0);
    let desired = match columns_min {
        // A manual width in Columns is the session's minimum: the preview still fills wider
        // free space, so no gap opens beside the focused column.
        Some(_) => manual.map_or(free, |m| free.max(m)),
        None => manual.unwrap_or_else(|| free.min(rules.auto_max)),
    };
    let width = desired.clamp(min, max.max(min)).round();
    Solved {
        width,
        pane: width.max(rules.hide_below),
        // The slot with the preview is the divider's line and the pane, so the empty one is
        // too: dismissing the preview moves nothing.
        empty_slot: if width > 0.0 {
            (width + rules.line - trailing).max(0.0)
        } else {
            0.0
        },
        fits: room - keep_standard >= rules.hide_below,
        min: manual_floor,
        max: max.max(manual_floor),
    }
}

/// The right pane in Columns, for C2: how wide the space right of the navigated columns is,
/// whether the preview shows in it, and what the empty slot keeps once trailing columns have
/// borrowed their width. A trailing child column is clipped at the right edge when it is
/// wider than the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RightPane {
    pub width: f32,
    pub preview_shown: bool,
    pub empty_slot: f32,
}

/// [`RightPane`] for `state` in a split `available` points wide.
pub fn right_pane(
    state: &super::DrawerState,
    rules: &DrawerRules,
    fit: &ColumnsFit,
    available: f32,
) -> RightPane {
    let solved = solve(
        rules,
        &Arrangement::Columns(*fit),
        available,
        state.manual_width(),
    );
    RightPane {
        width: solved.width,
        preview_shown: state.is_open() && solved.fits,
        empty_slot: solved.empty_slot,
    }
}

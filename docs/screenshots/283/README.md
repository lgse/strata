# Directory loading grace period (#283)

`before.png` (base `c779695`) and `after.png` show List view approximately
40 ms after navigation, with directory enumeration held by a synthetic source.
Before, the skeleton is already visible; after, the pane stays clear during the
150 ms grace period. Completed listings and slow-load skeleton styling are unchanged.

The ignored `ui::browser::tests::loading::capture_loading_frame` fixture captures
this state when `STRATA_LOADING_CAPTURE` names an output PNG. Run it on a private
Xvfb display with disposable XDG directories and `GSK_RENDERER=cairo`.

# Directory loading grace period (#283)

`before.png` (base `c779695`) and `after.png` show List view approximately
40 ms after navigation, with directory enumeration held by a synthetic source.
Before, the skeleton is already visible; after, the pane stays clear during the
150 ms grace period. Completed listings and slow-load skeleton styling are unchanged.

These are historical captures. The one-off screenshot generator was retired in
#837; `ui::browser::tests::loading::directory_loading_grace_across_modes` retains
the fast/slow load, reload, failure, empty, and superseded-load regression checks.

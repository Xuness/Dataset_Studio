# Frontend usability 0.5

Status: complete, 2026-09-08. Primary design and acceptance resolution: 2560 × 1440.

The user approved the findings in `.local/reports/frontend-ux-20260908/REPORT.md`
and requested multiple-rating OR, positive multi-tag matching and multi-tag exclusion.
This work improves existing surfaces without introducing new feature modules.

- [x] Separate metadata inspection (256 MiB), configurable bulk queries (default 12 GiB) and temporary disk budgets (8 GiB); the original real Rating query returned 992,722 objects.
- [x] Versioned rating sets and tag all/any/none conditions, validation, NULL semantics and source execution.
- [x] Quick filters for the explicit current browsing scope; saved definitions remain a deliberate action.
- [x] Batched Danbooru identifiers, ambiguous/missing/unavailable summaries, readable card captions and identity copying.
- [x] Consistent view navigation and query panel ownership; retain drafts and bounded browsing history.
- [x] Single-image previous/next and keyboard navigation, page/focus consistency and range-change cursors.
- [x] Shift range selection and deselection through thumbnails, checkboxes and Shift+Space; the endpoint's current state determines the operation.
- [x] Unified native title/menu bar and canonical Ds vector icon, native window operations.
- [x] 2560 × 1440 typography, proportional thumbnails, form widths and resizable panels.
- [x] Keyboard menus, scope-aware empty states, consistent rating names, actionable error summaries and copyable details.
- [x] Contracts, static checks, focused regressions, integration and native UI verification; daily app upgraded to 0.5.0 with its project preserved.

The daily project was backed up before closing the development window. Unrelated
project selections, query history, tool drafts and artifacts must be preserved.

Filter semantics: selected ratings match any code. Included tags support all or
any matching; an exclusion list rejects a row containing any listed tag. All
metadata clauses apply to the same observation under the chosen observation rule.
Missing tag metadata is unknown, not an empty known tag set.

Verification: `.local/reports/frontend-ux-20260908/VERIFICATION.md`. The project
upgrade from database v4 to v5 preserved all 23 non-migration tables, including
the exact existing query/tool/session drafts, selection and workset members.
The migration created and verified its own rollback backup. Original analysis
remains in REPORT.md for comparison; generated evidence stays outside Git.

# Incremental query reuse and Danbooru ordering 0.6

Status: complete. Daily application upgraded to 0.6.0 on 2026-09-08.

The daily archive appends observations/assets with commit_seq and updates
current_posts for affected posts. Catalog and analysis watermarks must agree.
The application must keep this archive read-only, including its journal.

- [x] Validate the producer's append/refresh contract and derive affected SHA
  identities from committed SSD metadata, including old and new post associations,
  new physical objects and missing-post observations. Reject incomplete histories.
- [x] Maintain one application-owned, versioned Danbooru browse index per source;
  support post ID ordering with bounded keyset paging and deterministic ties.
- [x] Share immutable membership versions between equivalent query executions;
  exclude presentation order from the membership fingerprint.
- [x] Refresh reusable memberships from affected identities instead of copying
  every unchanged member; publish only after consistency checks succeed.
- [x] Replace small durable insertion transactions with a bounded staging store
  and ordered bulk publication, retaining cancellation and failure recovery.
- [x] Bound unreferenced query retention by capacity, age and current use. Keep
  selection/workset/job references safe and expose cleanup/resource state.
- [x] Add sort controls, cache/reuse/refresh feedback, stale-source refresh and
  resource settings in the existing UI at 2560 x 1440.
- [x] Verify real first/repeated query I/O, synthetic multi-day updates and old
  post changes, missing history fallback, eviction, pinned snapshots, migration,
  SDK/native UI behavior and the full required checks.
- [x] Upgrade the daily application after isolated validation; preserve the
  user's current project, drafts, query history and saved members.

Pre-development backup: `.local/query-incremental-1788870146342/backup`.
Daily native window closed normally before source edits; testing uses isolated
application directories. No source archive mutation or Git push is included.

Validation: pnpm check (68 Rust tests and SDK checks), all six integration scripts,
Web/native builds and native UI checks passed. Migration preserved all 22 original
business tables. Evidence: .local/reports/query-incremental-20260908/REPORT.md.

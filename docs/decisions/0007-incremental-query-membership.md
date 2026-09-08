# 0007 — Incremental query membership and Danbooru ordering

Accepted for 0.6.0, 2026-09-08. Project database version 6; manifest version 1.

Daily ingestion changes both new objects and older post observations or asset
associations. Treating each catalog watermark as an independent materialized
query caused repeated writes and retained redundant member lists.

The source archive remains read-only. Each attached Danbooru library has one
application-owned browse index, keyed by SHA-256 with the lowest associated post
ID. It uses indexed keyset paging, places missing post IDs last in both directions,
and breaks ties by source and SHA in the requested direction. Source browsing
merges at most eight bounded pages. Selection and workset ordering use internal
query results, excluded from user query history and protected while being viewed.
Index preparation runs in the background and the browser shows a preparing state.

The producer contract is append-only observations and assets with commit_seq,
applied commit sequence and batch identity, and a catalog watermark matching the
analysis watermark. Changes are derived from observations, assets, affected posts,
old and new associations, and new physical objects. Reliable change anchors require
the same generation, the original anchor batch and a continuous commit sequence.
Anchors are checked against the cached result's generation as well as its sequence;
equal sequence numbers after a rebuild do not establish continuity. Missing history
or a generation replacement triggers full evaluation of the requested result.

Normalized conditions, sources, observation rules, and fixed input scope identify
a membership family. Presentation order and the advancing source-scope revision
do not create another family. Selection revisions, workset identities and query
input references remain part of the meaning. Compatible requests share a family
version. Changed identities close or add validity intervals; unchanged identities
are retained in place. A repeated query creates only its result record and dependency
references. Separate result records retain the requested order and definition.

Results are built in a disposable SQLite staging database and published in one
durable project transaction. Bounded streaming, ordered publication, cancellation,
and recovery remain in place. Project WAL and FULL synchronization are retained;
only unpublished scratch data uses relaxed durability. Both source versions and
mutable input scope are fenced before publication. A cancelled or interrupted
revision is rolled back without changing the prior fixed membership.

Default query working memory is 12 GiB, configurable from 1 to 64 GiB. At the
default, native execution receives 10 GiB and result staging receives up to 2 GiB;
publication uses a bounded SQLite pager after native execution ends. Native spill
and result staging each have an 8 GiB disk limit. Shared index construction budgets
its SQLite cache and chooses memory or file sorting according to the configured
budget. These are work budgets, not an OS limit on total process memory. Completed
and abandoned application-owned scratch files are removed. The sources themselves
are never used for query scratch.

Unreferenced reuse defaults to 4 GiB, seven days since last use and at most 256
families per project. A global catalog also accounts for closed projects. Cleanup
holds the ordinary project lease and does not change the daily recent-project
registry. It processes bounded batches; current views have renewable leases.
Selections, worksets, jobs and other durable references protect the exact versions
they need. Old versions without users are pruned. An index over expired member
intervals means pruning a few daily changes does
not scan every unchanged member or repeat work for every reused result record.
A zero retention setting also collects newly completed uncached results after
their viewers leave. Protected
members and shared source indexes are reported separately from retention policy;
protected data can exceed the configured retention capacity.

Version 6 migrates result_members to versioned storage behind a compatibility
view. Original result records, definitions, drafts, selections, worksets, jobs and
artifacts are preserved. Migration keeps a verified pre-upgrade backup. New and
successfully migrated projects use incremental auto-vacuum so evicted pages can
be returned to disk in bounded batches. Compaction is independent of query success;
if the optional migration compaction fails, the schema upgrade remains usable.

Fixed membership does not imply frozen source metadata. A query against an old
source version is marked stale and must be refreshed before use as a current query.
The UI explicitly rebinds a source scope to its latest revision when refreshing,
preserves the filter draft, and reports reuse, incremental refresh or full fallback.
Existing saved worksets retain their original members.

Validation includes source-level old-post/relink/orphan cases, bounded deep paging,
shared versions and recovery, repeated-day pruning, migration preservation, global
cleanup with closed projects, seven HTTP integration scenarios, and the native UI
at 2560 × 1440. The repeatable producer fixture and independent expected-member
comparison live in tooling/integration-query-cache.mjs. Real-lake performance
evidence and daily upgrade checks are stored under the ignored .local directory.

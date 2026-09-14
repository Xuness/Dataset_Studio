# 0008 — Cache tiers and application settings

Accepted for 0.7.0, 2026-09-09. Project database version 7; manifest version 1.
This replaces the retention defaults in decision 0007, while keeping its versioned
membership, read-only source contract and publication guarantees.

The application has one settings dialog, opened from the Settings menu. Pages
are registered separately from business panels. Cache budgets and query memory
remain accessible without an open project. Draft edits survive navigation between
settings pages. Existing layouts referring to the resource panel restore the
browser; the compatibility resource view links to the appropriate settings page.

The default total retention budget is 64 GiB and can be raised to 1024 GiB. Long-term
queries and shared indexes receive 48 GiB; temporary queries receive 8 GiB. Existing
preview capacity is preserved (the preview service default is 2 GiB), so a new
development environment initially allocates 58 of its 64 GiB total. Users can
change each category; their budgets must fit within the total. Global accounting
includes project member storage, shared rating bases, browse indexes, previews and
their indexes. Authoritative project artifacts remain separate. Native spill,
unpublished result databases and rating rebuild files are shown as working space.
The total is a retention policy, not a filesystem hard limit.

Pure current-post rating conditions on a source scope default to long-term
retention, without an idle deadline. Tag combinations and other queries default
to temporary retention for 24 hours since last use. A configured idle deadline
does not promise a minimum lifetime: capacity pressure can evict unprotected
members sooner. Long-term idle expiry is optional. Both categories can be adjusted
or explicitly fixed in the manager without copying member rows. Fixed families,
live viewing leases and durable references prevent ordinary deletion. Shared browse
indexes are required for bounded source ordering and are accounted as protected
infrastructure. Individual query and category sizes are apportioned estimates;
the member table and its indexes are measured once for aggregate accounting.

Session-only temporary retention is an alternative to the idle deadline. Each
client creates a fresh identity when it opens a project, renews it every 20 seconds,
and retires it on normal close. Engine leases expire after 90 seconds without a
heartbeat. A live connection rebind preserves that identity; reopening a project
creates a new one. A family may have multiple using sessions. Before reuse or
restoring a browser result, the engine checks that a family session is still alive,
or that fixed/durable retention protects the result. Renewing a view cannot revive
an expired session-only result. Cleanup of its unreferenced storage follows in
bounded batches; project scope and filter drafts can remain so Apply computes it
again. Fixed worksets, selections and jobs retain their exact member versions.
Closing one client leaves the project available to its other live sessions. During
large publication transactions, query progress and session validity can read an
independent committed WAL snapshot, so polling does not wait for the writer or
observe partially published members.

Rating bases are application-owned SQLite files keyed by library identity and
rating, outside project databases. Format 2 records observation row identity, asset
SHA, post ID and nullable Tag text for current-post candidates. Sources sharing a library reuse one base
regardless of project or display name. Bases build on demand or through the manager's
four-rating prebuild action. A global query work permit bounds prebuild and query
execution. Pins protect bases through query publication; cancellation and ordinary
eviction respect those pins. A directory ownership marker and canonical containment
checks constrain cache edits and cleanup to application-owned paths.

Generation, commit sequence, batch identity and continuity fence each base. Daily
updates cover changed observations, post associations and physical object identities.
Missing history or a new generation rebuilds the affected base. Full rebuilds publish
an independent file only after completion; incremental updates use a rollback
transaction. Cancellation preserves the previous completed base. Unpublished orphan
files are recovered before the first build on an instance.

Pure ratings and common rating/Tag combinations evaluate directly in the selected
bases. Tag conditions use case-sensitive literal-space token boundaries, preserving
AND, OR, exclusion and NULL semantics. Tag payloads scan sequentially rather than
following the SHA index into random table pages. Each streamed source payload has a
128 KiB bound. A cancelled query or expired work budget interrupts the cached scan,
including scans producing no matches.

Eligible queries import matching observation IDs and binary image identities through
the bundled DuckDB C appender. Native sorting puts the filtered identities in SHA
order before catalog and browse-index lookup, under the query memory/spill budget.
Other metadata predicates select required source columns through a temporary
candidate relation and evaluate their original conditions, preserving same-record conjunction semantics.
It does not combine a rating from one post with tags from another post of the same
image. It uses the cached current-post association without rejoining the full source
identity tables. Historical queries and small explicit input scopes keep the existing path;
daily deltas evaluate only affected identities. Source external access remains
disabled, and scratch stays in the application directory. Candidate narrowing is
measured separately from exact-result reuse; its overhead and source-column I/O
mean it does not promise faster execution for every predicate. Common Tag fields
are retained because identity-only candidates still required repeated full source
column reads in the real-lake prototype.

Shared bases do not replace project result versions: bounded paging, worksets,
selection, tools and recovery continue to use project membership. This avoids
cross-project ownership or mutable-scope dependencies. Changing a cache's retention
does not change the query or its saved definition. Source-dependent queries still
require refresh when source versions change, including long-term or fixed cache
entries. Fixed project scopes evaluated only against saved artifact fields reuse
their membership across lake updates; see decision 0021 for the dependency boundary.

Version 7 adds family tier, fixed state, member counters, session references and
candidate usage metadata. Migration preserves existing results and business records,
classifies eligible pure rating families, and retains a consistent backup. Closed
registered projects are maintained under their ordinary project lease and can be
upgraded from version 6 for cache maintenance without changing the recent-project
registry. Explicit legacy cache settings are backed up to query-cache.legacy-v1.json
and migrated while retaining their total query capacity and expiry intention.

Validation covers multi-post conjunctions, shared bases, daily relinks and fallback,
session invalidity before physical cleanup, promotion without copying, referenced
and fixed retention, legacy configuration migration, bounded pruning, HTTP/SDK
reconnection and the native settings dialog. The repeatable HTTP fixture is
tooling/integration-cache-settings.mjs; the optional native check is
tooling/smoke-settings-ui.mjs. Local real-lake benchmarks, screenshots and the daily
upgrade comparison are kept in the ignored .local directory.

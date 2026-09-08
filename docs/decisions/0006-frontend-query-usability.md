# 0006 · Query and browsing usability

Status: implemented, 2026-09-08.

The existing browser and query surfaces share typed filter semantics. Query v1
remains readable; v2 adds text sets, tag all/any/none and an optional explicit
project input scope. Conditions remain a conjunction on one observation. Selected
ratings are a set OR. Exclusion rejects any listed tag; NULL metadata does not
become an empty known tag set. Tag and set values are canonical, bounded and
quoted by the controlled compiler.

Project database v5 is a compatibility marker for the new JSON query protocol;
existing tables and records are unchanged. The normal checked backup precedes
upgrade. Older engines reject the project version rather than misreading v2
definitions. Application preferences and prior drafts have additive defaults.

Quick filters use POST query-results without implicitly creating or updating a
saved definition. Project scopes are checked against their declared revision.
Query inputs retain references to underlying results during execution. Mutable
selection revisions are checked per batch and in the publication transaction;
changing a selection fails the old query rather than silently changing its input.
Published results have fixed membership and are independent of later selection
changes. Small scopes (up to 4096 objects) use bounded identity predicates;
larger scopes intersect a source stream through indexed project membership.

Metadata inspection reserves 256 MiB. Bulk queries default to 12 GiB, allow at most
8 GiB of temporary disk space in an application-owned per-session directory and
retain cancellation/time limits. The class budget leaves room for metadata while
a bulk query runs. Closing native handles precedes removing temporary files.
These limits are reported separately from the thumbnail cache quota.
The query-memory setting is an integer from 1 to 64 GiB, persisted in the engine's
application directory. A running query keeps one immutable budget snapshot for
admission and all native sessions. Edits made during that query apply to the next
query; the coordinator changes its byte budget after the previous lease ends.
These are reservations and native working-memory limits, not preallocations or
an operating-system limit on the whole process. A test deliberately sets 32 MiB
to force real external sorting and prove temporary-file cleanup.

List titles use a batched metadata identity summary. SHA-256 remains the stored
object identity, while up to eight linked post IDs and an exact link count support
presentation. Unlinked metadata and temporarily unavailable metadata are distinct.

Navigation selects a central view; opening the query panel always accompanies the
browser. Parameter drafts survive view changes. Browser history remains bounded
by both count and bytes. The main layout is designed and verified at 2560 × 1440.

# 0009 — Population tools and MetaRecall ranking artifacts

Accepted for 0.8.0, 2026-09-09. Project database version 8; manifest version 1.

Danbooru MetaRecall is a registered population operator. The application port
explicitly distinguishes population work from row operators. Source adapters
project metadata in bounded batches; the engine owns input freezing, persistence,
resource admission and publication. The numeric operator reads immutable input
material, without opening source indexes, image packs, HTTP or project databases.
Existing manifest and integer operators keep their original execution format.

All four project scope references are accepted: source, query result, workset and
selection. A submitted task freezes its membership, source versions and per-member
query branches. Query predicates and observation rules remain attached when a
result becomes a workset or selection. Each representative must satisfy a complete
branch for that member and describe the same stored bytes. Cross-post and cross-source
duplicates use one complete representative, respecting observation precision and
stable tie keys. The eligible population inside this scope is the reference for
every statistic, separately within each selected rating.

The input uses an independent typed SQLite material with compact binary hashes.
Rankings use a second typed SQLite material, indexed for rating, route, eligibility
and keyset pagination. A summary manifest is the third artifact file. Floating
features, ranks, input provenance and missingness do not pass through the legacy
integer JSONL projection. API pages return at most 128 rows; the React board uses
48 rows and retains at most 64 previous cursors. Workset creation executes a
bounded-memory INSERT SELECT in the engine and is idempotent. The resulting
ordinary collection holds artifact and original query-result references.

The scoring implementation uses exact integer heat keys and deterministic midranks,
calendar-month neighborhoods, date-precision-aware age cohorts and the documented
fixed fallback levels. A Fenwick tree evaluates sliding cohorts. Artist support is
optional, scoped, self-excluding and parent-group limited. Quotas use integer
thousandths and the largest-remainder method; seeded SHA-256 domains separate score
ties from audit sampling. Stored dimensions are projected only when the optional
purpose gate requests them; origin metadata is used as a fallback and source
dimensions are never substituted for the stored image dimensions.

The worker loads one eligible rating at a time. Numeric working-set admission uses
the configured native query budget capped at 4 GiB and a conservative per-candidate
estimate; these are admission estimates, not whole-process RSS guarantees. Source
projection permits 8 GiB native spill and an hour-long query deadline. Individual
task materials have a 64 GiB limit. Checkpoints preserve completed ratings; an
interrupted rating is rebuilt from immutable input. Input SHA, coverage, feature
ranges, score formulas, rank permutations and exact quotas are checked before
publication. Failed worker messages are recorded separately from progress and
returned with the task error. Cancellation and process restart never publish an
incomplete result. Prepared tasks can resume while their original source is offline.

Three artifact files publish with hashes and durable references. Published input
material is retained even after the redundant staging link is removed. Artifact
release respects worksets, selections and downstream jobs. Durable ranking artifacts
are not evictable query cache. Database v8 adds artifact table registration, workset
idempotency and task stage progress; upgrades first create a consistent backup,
including committed WAL records, and retain all earlier migration entries.

The tools view provides configuration, rankings and diagnostics. Its navigation and
ranking draft have separate instance identities, preserving the old basic-tool
draft. Historical task parameters remain immutable. Metadata scores express review
priority; computational correctness does not establish visual quality or recall.

The interface follows the supplied Photoshop reference: compact option bars, square
collapsible parameter sections, a dark work surface and docked properties. A view
can declare that it owns its inspector; the public UI context supplies the shared
visibility, width and resize controls. Ranking then fills that area with population
properties or the selected row's evidence, while the browser keeps its own asset
inspector. The result header and pagination remain fixed around the scrolling rows.
Ranking reports progress inline; the project task panel stays available on demand.

Validation evidence and current limits are recorded in
[the 0.8 verification report](../verification-metarecall-v0.8.md).

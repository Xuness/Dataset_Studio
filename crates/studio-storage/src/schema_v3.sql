CREATE TABLE query_definitions (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, revision INTEGER NOT NULL,
    spec_json TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX queries_listing ON query_definitions(created_at DESC,id DESC);
CREATE TABLE query_results (
    id TEXT PRIMARY KEY, definition_id TEXT REFERENCES query_definitions(id),
    definition_revision INTEGER, spec_json TEXT NOT NULL, versions_json TEXT NOT NULL,
    status TEXT NOT NULL, processed INTEGER NOT NULL DEFAULT 0, count INTEGER,
    created_at TEXT NOT NULL, error TEXT
);
CREATE INDEX results_scheduling ON query_results(status, created_at);
CREATE INDEX results_listing ON query_results(created_at DESC,id DESC);
CREATE TABLE result_members (
    result_id TEXT NOT NULL REFERENCES query_results(id),
    source_id TEXT NOT NULL REFERENCES sources(id), asset_id TEXT NOT NULL,
    PRIMARY KEY(result_id, source_id, asset_id)
) WITHOUT ROWID;
CREATE TABLE selection_base (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    result_id TEXT NOT NULL REFERENCES query_results(id)
);
CREATE TABLE selection_exclusions (
    source_id TEXT NOT NULL REFERENCES sources(id), asset_id TEXT NOT NULL,
    PRIMARY KEY(source_id, asset_id)
) WITHOUT ROWID;
CREATE TABLE result_references (
    owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    result_id TEXT NOT NULL REFERENCES query_results(id),
    PRIMARY KEY(owner_kind, owner_id, result_id)
) WITHOUT ROWID;
CREATE TABLE job_scopes (
    job_id TEXT PRIMARY KEY REFERENCES jobs(id), scope_json TEXT NOT NULL,
    provenance_json TEXT NOT NULL, result_id TEXT REFERENCES query_results(id)
);
CREATE TABLE collection_scopes (
    collection_id TEXT PRIMARY KEY REFERENCES collections(id),
    scope_json TEXT NOT NULL, provenance_json TEXT NOT NULL
);

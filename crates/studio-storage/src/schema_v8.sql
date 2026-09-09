CREATE TABLE artifact_tables (
    artifact_id TEXT PRIMARY KEY REFERENCES artifacts(id),
    row_count INTEGER NOT NULL,
    table_sha256 TEXT NOT NULL,
    input_sha256 TEXT NOT NULL,
    summary_json TEXT NOT NULL
);
CREATE TABLE ranking_workset_requests (
    request_id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    request_json TEXT NOT NULL,
    collection_id TEXT NOT NULL REFERENCES collections(id)
);
CREATE TABLE job_progress (
    job_id TEXT PRIMARY KEY REFERENCES jobs(id),
    stage_json TEXT NOT NULL
);

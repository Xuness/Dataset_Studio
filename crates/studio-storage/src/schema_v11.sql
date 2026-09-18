CREATE TABLE evaluation_analysis_refs(id TEXT PRIMARY KEY, state TEXT NOT NULL);
-- Members are copied in short transactions while this build remains invisible.
-- Publication changes only state/count/provenance in one transaction.
CREATE TABLE evaluation_workset_builds (
 job_id TEXT PRIMARY KEY, collection_id TEXT UNIQUE REFERENCES collections(id) ON DELETE SET NULL,
 request_json TEXT NOT NULL, state TEXT NOT NULL, after_position INTEGER NOT NULL DEFAULT 0,
 count INTEGER NOT NULL DEFAULT 0
);

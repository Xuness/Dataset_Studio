CREATE TABLE evaluation_stage_refs (
 id TEXT PRIMARY KEY, collection_id TEXT REFERENCES collections(id),
 state TEXT NOT NULL, request_json TEXT NOT NULL, total INTEGER NOT NULL
);
CREATE TABLE evaluation_source_refs (
 stage_id TEXT NOT NULL REFERENCES evaluation_stage_refs(id),
 source_id TEXT NOT NULL REFERENCES sources(id), PRIMARY KEY(stage_id,source_id)
) WITHOUT ROWID;

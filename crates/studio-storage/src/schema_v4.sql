CREATE TABLE job_runs (
    job_id TEXT PRIMARY KEY REFERENCES jobs(id),
    run_json TEXT NOT NULL, fields_json TEXT NOT NULL, versions_json TEXT NOT NULL
);
CREATE TABLE artifacts (
    id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES jobs(id),
    output_id TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
    schema_version INTEGER NOT NULL, status TEXT NOT NULL, count INTEGER,
    created_at TEXT NOT NULL, files_json TEXT NOT NULL, provenance_json TEXT NOT NULL,
    issue TEXT, UNIQUE(job_id,output_id)
);
CREATE INDEX artifacts_listing ON artifacts(created_at DESC,id DESC);
CREATE TABLE artifact_rows (
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    source_id TEXT NOT NULL, asset_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    scalar_value INTEGER, scalar_status TEXT, row_json TEXT NOT NULL,
    PRIMARY KEY(artifact_id,source_id,asset_id)
) WITHOUT ROWID;
CREATE INDEX artifact_scalars ON artifact_rows(artifact_id,scalar_status,scalar_value,source_id,asset_id);
CREATE TABLE artifact_references (
    owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    PRIMARY KEY(owner_kind,owner_id,artifact_id)
) WITHOUT ROWID;
CREATE TABLE tool_drafts (
    module_id TEXT NOT NULL, instance_id TEXT NOT NULL,
    schema_version INTEGER NOT NULL, revision INTEGER NOT NULL,
    updated_at TEXT NOT NULL, value_json TEXT NOT NULL,
    PRIMARY KEY(module_id,instance_id)
) WITHOUT ROWID;
-- Existing results remain at their exact paths. File validation is lazy per artifact.
-- The producer version and input fields are unknown until supported evidence is read.
INSERT INTO artifacts(id,job_id,output_id,name,kind,schema_version,status,count,created_at,files_json,provenance_json)
SELECT id,id,'data','旧清单成果','manifest',1,'legacy',total,created_at,
       json_array(json_object('path',artifact,'bytes',NULL,'sha256',NULL,'media_type','application/x-ndjson')),
       json_object('run',NULL,'input_scope',NULL,'input_sha256',NULL,'attempt',NULL,'input_artifacts',json_array(),'fields_frozen',json('false'),'evidence','legacy_job_record; unverified_file')
FROM jobs WHERE status='succeeded' AND artifact IS NOT NULL;

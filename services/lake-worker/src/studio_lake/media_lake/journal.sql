-- New format-2 library journal. Not a migration for existing format-1 archives.
PRAGMA foreign_keys = ON;
PRAGMA application_id = 0x44534A4C;
PRAGMA user_version = 2;
CREATE TABLE commits(
  seq INTEGER PRIMARY KEY AUTOINCREMENT, batch_id TEXT NOT NULL UNIQUE,
  dedupe_key TEXT NOT NULL UNIQUE, manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json)),
  committed_at TEXT NOT NULL, UNIQUE(seq,batch_id)
);
CREATE TABLE progress(source_key TEXT PRIMARY KEY,next_row INTEGER NOT NULL,complete INTEGER NOT NULL DEFAULT 0);
CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE releases(release_id TEXT PRIMARY KEY,state TEXT NOT NULL,definition_json TEXT NOT NULL,
  remote_commit TEXT,verification_json TEXT);
CREATE TABLE collection_runs(
  job_id TEXT PRIMARY KEY, definition_sha256 TEXT NOT NULL,
  intent_batch_id TEXT NOT NULL REFERENCES commits(batch_id),
  planner_version TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE collection_run_batches(
  job_id TEXT NOT NULL REFERENCES collection_runs(job_id), seq INTEGER NOT NULL,
  batch_id TEXT NOT NULL, receipt_sha256 TEXT NOT NULL,
  PRIMARY KEY(job_id,seq), UNIQUE(seq),
  FOREIGN KEY(seq,batch_id) REFERENCES commits(seq,batch_id)
);
CREATE TABLE collection_checkpoints(
  job_id TEXT NOT NULL REFERENCES collection_runs(job_id), stream_key TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK(revision >= 0), cursor_json TEXT NOT NULL CHECK(json_valid(cursor_json)),
  seq INTEGER NOT NULL, batch_id TEXT NOT NULL,
  PRIMARY KEY(job_id,stream_key), FOREIGN KEY(seq,batch_id) REFERENCES commits(seq,batch_id)
);

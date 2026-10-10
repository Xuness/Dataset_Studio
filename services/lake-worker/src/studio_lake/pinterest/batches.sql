-- v22: immutable logical results and durable membership of physical archive batches.
CREATE TABLE pinterest_batches(batch_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),
 receipt_ids_json TEXT NOT NULL CHECK(json_valid(receipt_ids_json)),dedupe_key TEXT NOT NULL UNIQUE,
 state TEXT NOT NULL CHECK(state IN ('prepared','accepted','published','cancelled')),
 archive_seq INTEGER,created_at REAL NOT NULL,cleanup_complete INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_complete IN (0,1)));
CREATE INDEX pinterest_batches_pending ON pinterest_batches(job_id,state,created_at,batch_id);
CREATE INDEX pinterest_batches_cleanup ON pinterest_batches(job_id,cleanup_complete,state,batch_id);
CREATE TABLE pinterest_outbox(receipt_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),
 task_id TEXT NOT NULL REFERENCES pinterest_tasks(task_id),claim_token TEXT NOT NULL,
 version INTEGER NOT NULL CHECK(version IN (1,2)),definition_sha256 TEXT NOT NULL,input_sha256 TEXT NOT NULL,
 content_sha256 TEXT NOT NULL,replay_json TEXT NOT NULL CHECK(json_valid(replay_json)),
 row_count INTEGER NOT NULL CHECK(row_count>=0),metadata_bytes INTEGER NOT NULL CHECK(metadata_bytes>=0),
 media_bytes INTEGER NOT NULL CHECK(media_bytes>=0),media_records INTEGER NOT NULL CHECK(media_records>=0),
 download_key TEXT,context_key TEXT,checkpoint_key TEXT,
 batch_id TEXT REFERENCES pinterest_batches(batch_id),
 state TEXT NOT NULL CHECK(state IN ('prepared','accepted','published','cancelled')),
 cleanup_complete INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_complete IN (0,1)),
 created_at REAL NOT NULL,archive_seq INTEGER);
CREATE INDEX pinterest_outbox_pending ON pinterest_outbox(job_id,state,created_at,receipt_id);
CREATE INDEX pinterest_outbox_batch ON pinterest_outbox(batch_id,receipt_id);
CREATE INDEX pinterest_outbox_cleanup ON pinterest_outbox(job_id,cleanup_complete,state,receipt_id);
CREATE UNIQUE INDEX pinterest_outbox_task_claim ON pinterest_outbox(task_id) WHERE state='prepared';

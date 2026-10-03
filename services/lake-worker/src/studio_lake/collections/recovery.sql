-- v11: immutable logical receipts, separately frozen physical batch membership.
ALTER TABLE collection_tasks ADD COLUMN download_generation INTEGER NOT NULL DEFAULT 0 CHECK(download_generation>=0);
CREATE TABLE collection_outbox_next(
  id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  task_id TEXT, execution_epoch INTEGER NOT NULL, claim_token TEXT,
  dedupe_key TEXT NOT NULL UNIQUE, intent_path TEXT NOT NULL, content_sha256 TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('prepared','archive_committed','released','needs_review','quarantined')),
  archive_seq INTEGER, batch_id TEXT,
  control_applied INTEGER NOT NULL DEFAULT 0 CHECK(control_applied IN (0,1)),
  published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1)),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  outcome_state TEXT,
  FOREIGN KEY(task_id,job_id) REFERENCES collection_tasks(id,job_id),
  CHECK((control_applied=0 AND published=0) OR (archive_seq IS NOT NULL AND batch_id IS NOT NULL)),
  CHECK(state <> 'released' OR (control_applied=1 AND published=1)),
  CHECK(state <> 'quarantined' OR (control_applied=0 AND published=0 AND archive_seq IS NULL))
) WITHOUT ROWID;
INSERT INTO collection_outbox_next SELECT *,NULL FROM collection_outbox;
DROP TABLE collection_outbox;
ALTER TABLE collection_outbox_next RENAME TO collection_outbox;
CREATE INDEX collection_outbox_pending ON collection_outbox(job_id,state,created_at,id);
CREATE INDEX collection_outbox_batch ON collection_outbox(batch_id,id);
CREATE TABLE collection_batches(
  id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  receipt_ids_json TEXT NOT NULL CHECK(json_valid(receipt_ids_json)),
  dedupe_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK(state IN ('prepared','archive_committed','quarantined')),
  archive_seq INTEGER, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX collection_batch_pending ON collection_batches(job_id,state,created_at,id);
CREATE TABLE collection_quarantines(
  receipt_id TEXT PRIMARY KEY REFERENCES collection_outbox(id),
  reason_code TEXT NOT NULL, evidence_path TEXT NOT NULL, expected_sha256 TEXT NOT NULL,
  observed_sha256 TEXT, detected_at TEXT NOT NULL,
  replacement_receipt_id TEXT REFERENCES collection_outbox(id)
) WITHOUT ROWID;

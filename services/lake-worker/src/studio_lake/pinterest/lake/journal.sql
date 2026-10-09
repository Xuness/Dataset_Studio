PRAGMA foreign_keys=ON;
PRAGMA application_id=0x44534A4C;
PRAGMA user_version=3;
CREATE TABLE commits(seq INTEGER PRIMARY KEY AUTOINCREMENT,batch_id TEXT NOT NULL UNIQUE,dedupe_key TEXT NOT NULL UNIQUE,
 manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json)),committed_at TEXT NOT NULL);
CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE pinterest_runs(job_id TEXT PRIMARY KEY,definition_sha256 TEXT NOT NULL,definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 intent_batch_id TEXT NOT NULL REFERENCES commits(batch_id),created_at TEXT NOT NULL);
CREATE TABLE pinterest_receipts(receipt_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES pinterest_runs(job_id),
 seq INTEGER NOT NULL REFERENCES commits(seq),task_id TEXT,claim_token TEXT,
 replay_json TEXT NOT NULL CHECK(json_valid(replay_json)));
CREATE INDEX pinterest_receipts_job ON pinterest_receipts(job_id,seq);

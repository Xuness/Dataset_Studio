CREATE TABLE raw_receipts (
 attempt_id TEXT PRIMARY KEY REFERENCES attempts(id), metadata TEXT NOT NULL,
 body BLOB NOT NULL CHECK(length(body)<=16777216), sha256 TEXT NOT NULL CHECK(length(sha256)=64),
 saved_at TEXT NOT NULL
);
CREATE TABLE receipt_parses (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, attempt_id TEXT NOT NULL REFERENCES attempts(id),
 adapter_version TEXT NOT NULL, state TEXT NOT NULL, error TEXT, created_at TEXT NOT NULL
);
CREATE INDEX receipt_parse_attempt ON receipt_parses(attempt_id,sequence);
CREATE TABLE batch_replacements (
 parent INTEGER NOT NULL REFERENCES batches(sequence), child INTEGER NOT NULL UNIQUE REFERENCES batches(sequence),
 reason TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY(parent,child)
);
PRAGMA user_version=5;

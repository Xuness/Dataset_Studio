-- Display name and tombstone for offline analysis jobs. request_json stays frozen so
-- idempotent retries and provenance keep the original request; rows are retained so
-- comparisons, reviews and derived worksets that cite a removed job stay readable.
ALTER TABLE analysis_jobs ADD COLUMN name TEXT;
ALTER TABLE analysis_jobs ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1));
PRAGMA user_version=11;

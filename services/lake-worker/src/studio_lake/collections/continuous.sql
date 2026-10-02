-- Control v9: recurring snapshots and per-task retained/directory evidence.
ALTER TABLE collection_tasks ADD COLUMN summary_json TEXT CHECK(summary_json IS NULL OR json_valid(summary_json));
CREATE TABLE collection_schedules(
  id TEXT PRIMARY KEY, definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
  lake_id TEXT NOT NULL REFERENCES collection_lakes(lake_id),
  every_seconds INTEGER NOT NULL CHECK(every_seconds BETWEEN 60 AND 31622400),
  next_at REAL NOT NULL, enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
  revision INTEGER NOT NULL CHECK(revision>0), last_job TEXT REFERENCES collection_jobs(id),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX collection_schedule_due ON collection_schedules(enabled,next_at);
CREATE INDEX collection_job_definition ON collection_jobs(lake_id,definition_sha256,state);
CREATE INDEX collection_job_created ON collection_jobs(created_at,id);
CREATE INDEX update_job_created ON jobs(created_at,id);

CREATE TABLE analysis_jobs (
 id TEXT PRIMARY KEY, created_at TEXT NOT NULL, state TEXT NOT NULL,
 phase TEXT NOT NULL, progress INTEGER NOT NULL DEFAULT 0, total INTEGER NOT NULL DEFAULT 0,
 request_json TEXT NOT NULL, input_json TEXT NOT NULL, result_json TEXT, error TEXT,
 experiment_id TEXT
);
CREATE INDEX analysis_experiment ON analysis_jobs(experiment_id,id);
CREATE TABLE ranking_rows (
 snapshot_id TEXT NOT NULL REFERENCES analysis_jobs(id), position INTEGER NOT NULL,
 ordinal INTEGER NOT NULL, source_id TEXT NOT NULL, asset_id TEXT NOT NULL, rating TEXT NOT NULL,
 content_version TEXT NOT NULL, data TEXT NOT NULL,
 PRIMARY KEY(snapshot_id,position), UNIQUE(snapshot_id,ordinal), UNIQUE(snapshot_id,source_id,asset_id)
);
CREATE INDEX ranking_rating ON ranking_rows(snapshot_id,rating,position);
CREATE TABLE comparison_rows (
 job_id TEXT NOT NULL REFERENCES analysis_jobs(id), position INTEGER NOT NULL, data TEXT NOT NULL,
 PRIMARY KEY(job_id,position)
) WITHOUT ROWID;
CREATE TABLE experiments (
 id TEXT PRIMARY KEY, created_at TEXT NOT NULL, request_json TEXT NOT NULL, inputs_json TEXT NOT NULL
);
CREATE TABLE reviews (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, request_id TEXT NOT NULL UNIQUE,
 snapshot_id TEXT NOT NULL REFERENCES analysis_jobs(id), ordinal INTEGER NOT NULL,
 created_at TEXT NOT NULL, decision TEXT NOT NULL, request_json TEXT NOT NULL
);
CREATE INDEX review_candidate ON reviews(snapshot_id,ordinal,sequence DESC);
CREATE INDEX review_snapshot ON reviews(snapshot_id,sequence);
PRAGMA user_version=2;

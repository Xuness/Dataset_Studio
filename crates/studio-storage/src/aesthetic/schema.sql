CREATE TABLE stages (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL,
 config TEXT NOT NULL, config_hash TEXT NOT NULL, request_json TEXT NOT NULL,
 total INTEGER NOT NULL, frozen INTEGER NOT NULL DEFAULT 0, eligible INTEGER NOT NULL DEFAULT 0,
 attempts INTEGER NOT NULL DEFAULT 0, accepted INTEGER NOT NULL DEFAULT 0,
 invalid INTEGER NOT NULL DEFAULT 0, unknown INTEGER NOT NULL DEFAULT 0,
 protected INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL DEFAULT 0,
 output_tokens INTEGER NOT NULL DEFAULT 0, usage_unknown INTEGER NOT NULL DEFAULT 0,
 error TEXT
);
CREATE TABLE candidates (
 stage_id TEXT NOT NULL REFERENCES stages(id), ordinal INTEGER NOT NULL,
 source_id TEXT NOT NULL, asset_id TEXT NOT NULL, rating TEXT NOT NULL, year INTEGER,
 basis TEXT NOT NULL, content_version TEXT NOT NULL, bytes INTEGER NOT NULL,
 exposures INTEGER NOT NULL DEFAULT 0, protected INTEGER NOT NULL DEFAULT 0,
 reserved INTEGER NOT NULL DEFAULT 0, blocked INTEGER NOT NULL DEFAULT 0, sort_key TEXT NOT NULL,
 PRIMARY KEY(stage_id,ordinal), UNIQUE(stage_id,source_id,asset_id)
);
CREATE INDEX candidate_dispatch ON candidates(stage_id,blocked,reserved,exposures,sort_key);
CREATE INDEX candidate_rating ON candidates(stage_id,rating,blocked,reserved,exposures,sort_key);
CREATE INDEX candidate_protected ON candidates(stage_id,protected,ordinal);
CREATE TABLE batches (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, stage_id TEXT NOT NULL REFERENCES stages(id),
 rating TEXT NOT NULL, state TEXT NOT NULL, members TEXT NOT NULL,
 attempt_id TEXT, error TEXT, observation TEXT
);
CREATE INDEX batch_stage ON batches(stage_id,sequence);
CREATE INDEX batch_pending ON batches(stage_id,state,sequence);
CREATE TABLE attempts (
 id TEXT PRIMARY KEY, batch INTEGER NOT NULL REFERENCES batches(sequence),
 state TEXT NOT NULL, created_at TEXT NOT NULL, receipt TEXT, failure TEXT
);
CREATE INDEX attempt_batch ON attempts(batch,created_at,id);
CREATE INDEX attempt_received ON attempts(state,batch);
CREATE TABLE evidence (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 batch INTEGER NOT NULL UNIQUE REFERENCES batches(sequence),
 attempt_id TEXT NOT NULL UNIQUE REFERENCES attempts(id),
 observation TEXT NOT NULL, parser_version INTEGER NOT NULL, accepted_at TEXT NOT NULL
);
PRAGMA user_version=1;

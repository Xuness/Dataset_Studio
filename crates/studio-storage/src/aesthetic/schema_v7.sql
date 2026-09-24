CREATE TABLE sampling_plans (
 id TEXT PRIMARY KEY, stage_id TEXT NOT NULL REFERENCES stages(id),
 request_json TEXT NOT NULL CHECK(json_valid(request_json)), created_at TEXT NOT NULL,
 initial_watermark INTEGER NOT NULL, status_json TEXT NOT NULL CHECK(json_valid(status_json))
);
ALTER TABLE stages ADD COLUMN sampling_plan_id TEXT REFERENCES sampling_plans(id);
ALTER TABLE batches ADD COLUMN sampling TEXT CHECK(sampling IS NULL OR json_valid(sampling));
CREATE TABLE sampling_rounds (
 plan_id TEXT NOT NULL REFERENCES sampling_plans(id), round INTEGER NOT NULL CHECK(round>0),
 evidence_watermark INTEGER NOT NULL, summary_json TEXT NOT NULL CHECK(json_valid(summary_json)),
 PRIMARY KEY(plan_id,round)
);
CREATE TABLE sampling_queue (
 plan_id TEXT NOT NULL, round INTEGER NOT NULL, slot INTEGER NOT NULL,
 members TEXT NOT NULL CHECK(json_valid(members)), batch INTEGER UNIQUE REFERENCES batches(sequence),
 PRIMARY KEY(plan_id,round,slot), FOREIGN KEY(plan_id,round) REFERENCES sampling_rounds(plan_id,round)
);
CREATE INDEX sampling_unclaimed ON sampling_queue(plan_id,round,batch,slot);
CREATE TABLE sampling_diagnostics (
 plan_id TEXT NOT NULL, round INTEGER NOT NULL, ordinal INTEGER NOT NULL,
 data TEXT NOT NULL CHECK(json_valid(data)), PRIMARY KEY(plan_id,round,ordinal),
 FOREIGN KEY(plan_id,round) REFERENCES sampling_rounds(plan_id,round)
);
PRAGMA user_version=7;

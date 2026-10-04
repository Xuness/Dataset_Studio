ALTER TABLE stages ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1));
ALTER TABLE stages ADD COLUMN execution_settings TEXT CHECK(execution_settings IS NULL OR json_valid(execution_settings));
ALTER TABLE stages ADD COLUMN failure_streak INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stages ADD COLUMN next_batch_number INTEGER NOT NULL DEFAULT 1;
CREATE INDEX stage_recent ON stages(archived,created_at DESC,id DESC);
CREATE INDEX analysis_stage_latest ON analysis_jobs(json_extract(input_json,'$.stage_id'),state,json_extract(request_json,'$.spec.kind'),created_at DESC,id DESC);
CREATE TABLE execution_updates (
 stage_id TEXT NOT NULL REFERENCES stages(id), id TEXT NOT NULL, request_json TEXT NOT NULL,
 settings TEXT NOT NULL CHECK(json_valid(settings)), created_at TEXT NOT NULL,
 PRIMARY KEY(stage_id,id)
);
ALTER TABLE batches ADD COLUMN stage_sequence INTEGER NOT NULL DEFAULT 0;
ALTER TABLE batches ADD COLUMN retry_at INTEGER;
ALTER TABLE batches ADD COLUMN recovery_deadline INTEGER;
ALTER TABLE batches ADD COLUMN recovery_attempt_base INTEGER NOT NULL DEFAULT 0;
ALTER TABLE batches ADD COLUMN disposition_reason TEXT;
WITH numbered AS (SELECT sequence,ROW_NUMBER() OVER(PARTITION BY stage_id ORDER BY sequence) n FROM batches)
UPDATE batches SET stage_sequence=(SELECT n FROM numbered WHERE numbered.sequence=batches.sequence);
UPDATE stages SET next_batch_number=COALESCE((SELECT MAX(stage_sequence)+1 FROM batches WHERE stage_id=stages.id),1);
CREATE UNIQUE INDEX batch_local_number ON batches(stage_id,stage_sequence) WHERE stage_sequence>0;
CREATE INDEX batch_retry_due ON batches(stage_id,state,retry_at,sequence);
CREATE INDEX batch_sampling_round ON batches(stage_id,json_extract(sampling,'$.plan_id'),json_extract(sampling,'$.round'),state);
CREATE TRIGGER batch_number_insert AFTER INSERT ON batches WHEN NEW.stage_sequence=0 BEGIN
 UPDATE batches SET stage_sequence=(SELECT next_batch_number FROM stages WHERE id=NEW.stage_id) WHERE sequence=NEW.sequence;
 UPDATE stages SET next_batch_number=next_batch_number+1 WHERE id=NEW.stage_id;
END;
CREATE TABLE batch_state_counts (
 stage_id TEXT NOT NULL REFERENCES stages(id), state TEXT NOT NULL, count INTEGER NOT NULL CHECK(count>=0),
 PRIMARY KEY(stage_id,state)
) WITHOUT ROWID;
INSERT INTO batch_state_counts SELECT stage_id,state,COUNT(*) FROM batches GROUP BY stage_id,state;
CREATE TABLE batch_round_counts (
 plan_id TEXT NOT NULL, round INTEGER NOT NULL, state TEXT NOT NULL, count INTEGER NOT NULL CHECK(count>=0),
 PRIMARY KEY(plan_id,round,state)
) WITHOUT ROWID;
INSERT INTO batch_round_counts SELECT json_extract(sampling,'$.plan_id'),json_extract(sampling,'$.round'),state,COUNT(*)
 FROM batches WHERE sampling IS NOT NULL GROUP BY json_extract(sampling,'$.plan_id'),json_extract(sampling,'$.round'),state;
CREATE TRIGGER batch_counts_insert AFTER INSERT ON batches BEGIN
 INSERT INTO batch_state_counts VALUES(NEW.stage_id,NEW.state,1)
 ON CONFLICT(stage_id,state) DO UPDATE SET count=count+1;
 INSERT INTO batch_round_counts SELECT json_extract(NEW.sampling,'$.plan_id'),json_extract(NEW.sampling,'$.round'),NEW.state,1 WHERE NEW.sampling IS NOT NULL
 ON CONFLICT(plan_id,round,state) DO UPDATE SET count=count+1;
END;
CREATE TRIGGER batch_counts_update AFTER UPDATE OF state ON batches WHEN OLD.state!=NEW.state BEGIN
 UPDATE batch_state_counts SET count=count-1 WHERE stage_id=OLD.stage_id AND state=OLD.state;
 INSERT INTO batch_state_counts VALUES(NEW.stage_id,NEW.state,1)
 ON CONFLICT(stage_id,state) DO UPDATE SET count=count+1;
 UPDATE batch_round_counts SET count=count-1 WHERE plan_id=json_extract(OLD.sampling,'$.plan_id') AND round=json_extract(OLD.sampling,'$.round') AND state=OLD.state;
 INSERT INTO batch_round_counts SELECT json_extract(NEW.sampling,'$.plan_id'),json_extract(NEW.sampling,'$.round'),NEW.state,1 WHERE NEW.sampling IS NOT NULL
 ON CONFLICT(plan_id,round,state) DO UPDATE SET count=count+1;
END;
ALTER TABLE sampling_rounds ADD COLUMN planned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sampling_rounds ADD COLUMN unclaimed INTEGER NOT NULL DEFAULT 0;
UPDATE sampling_rounds SET
 planned=(SELECT COUNT(*) FROM sampling_queue q WHERE q.plan_id=sampling_rounds.plan_id AND q.round=sampling_rounds.round),
 unclaimed=(SELECT COUNT(*) FROM sampling_queue q WHERE q.plan_id=sampling_rounds.plan_id AND q.round=sampling_rounds.round AND q.batch IS NULL);
CREATE TRIGGER sampling_queue_counts_insert AFTER INSERT ON sampling_queue BEGIN
 UPDATE sampling_rounds SET planned=planned+1,unclaimed=unclaimed+(NEW.batch IS NULL) WHERE plan_id=NEW.plan_id AND round=NEW.round;
END;
CREATE TRIGGER sampling_queue_counts_delete AFTER DELETE ON sampling_queue BEGIN
 UPDATE sampling_rounds SET planned=planned-1,unclaimed=unclaimed-(OLD.batch IS NULL) WHERE plan_id=OLD.plan_id AND round=OLD.round;
END;
CREATE TRIGGER sampling_queue_counts_claim AFTER UPDATE OF batch ON sampling_queue BEGIN
 UPDATE sampling_rounds SET unclaimed=unclaimed+(NEW.batch IS NULL)-(OLD.batch IS NULL) WHERE plan_id=NEW.plan_id AND round=NEW.round;
END;
ALTER TABLE candidates ADD COLUMN blocked_batch INTEGER REFERENCES batches(sequence);
CREATE TEMP TABLE blocked_members AS
 SELECT b.stage_id,json_extract(m.value,'$.candidate.ordinal') ordinal,MAX(b.sequence) batch
 FROM batches b,json_each(b.members) m
 WHERE b.state IN ('failed','invalid','outcome_unknown','sent')
 GROUP BY b.stage_id,json_extract(m.value,'$.candidate.ordinal');
CREATE UNIQUE INDEX blocked_members_lookup ON blocked_members(stage_id,ordinal);
UPDATE candidates SET blocked_batch=(SELECT batch FROM blocked_members b WHERE b.stage_id=candidates.stage_id AND b.ordinal=candidates.ordinal)
 WHERE blocked=1 AND EXISTS(SELECT 1 FROM blocked_members b WHERE b.stage_id=candidates.stage_id AND b.ordinal=candidates.ordinal);
DROP TABLE blocked_members;
CREATE INDEX candidate_blocked ON candidates(stage_id,blocked,ordinal);
CREATE TABLE exposure_counts (
 stage_id TEXT NOT NULL REFERENCES stages(id), exposures INTEGER NOT NULL,
 disposition TEXT NOT NULL, blocked INTEGER NOT NULL, count INTEGER NOT NULL CHECK(count>=0),
 PRIMARY KEY(stage_id,exposures,disposition,blocked)
) WITHOUT ROWID;
INSERT INTO exposure_counts SELECT stage_id,exposures,disposition,blocked,COUNT(*) FROM candidates GROUP BY stage_id,exposures,disposition,blocked;
CREATE TRIGGER exposure_counts_insert AFTER INSERT ON candidates BEGIN
 INSERT INTO exposure_counts VALUES(NEW.stage_id,NEW.exposures,NEW.disposition,NEW.blocked,1)
 ON CONFLICT(stage_id,exposures,disposition,blocked) DO UPDATE SET count=count+1;
END;
CREATE TRIGGER exposure_counts_update AFTER UPDATE OF exposures,disposition,blocked ON candidates
 WHEN OLD.exposures!=NEW.exposures OR OLD.disposition!=NEW.disposition OR OLD.blocked!=NEW.blocked BEGIN
 UPDATE exposure_counts SET count=count-1 WHERE stage_id=OLD.stage_id AND exposures=OLD.exposures AND disposition=OLD.disposition AND blocked=OLD.blocked;
 INSERT INTO exposure_counts VALUES(NEW.stage_id,NEW.exposures,NEW.disposition,NEW.blocked,1)
 ON CONFLICT(stage_id,exposures,disposition,blocked) DO UPDATE SET count=count+1;
END;
ALTER TABLE attempts ADD COLUMN execution_settings TEXT;
CREATE TABLE batch_actions (
 id TEXT PRIMARY KEY, stage_id TEXT NOT NULL REFERENCES stages(id), request_json TEXT NOT NULL,
 upper_sequence INTEGER NOT NULL, after_sequence INTEGER NOT NULL DEFAULT 0,
 processed INTEGER NOT NULL DEFAULT 0, succeeded INTEGER NOT NULL DEFAULT 0, failed INTEGER NOT NULL DEFAULT 0,
 completed INTEGER NOT NULL DEFAULT 0, last_items TEXT NOT NULL DEFAULT '[]'
);
PRAGMA user_version=8;

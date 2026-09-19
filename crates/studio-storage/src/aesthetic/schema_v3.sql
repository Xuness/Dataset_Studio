ALTER TABLE candidates ADD COLUMN disposition TEXT NOT NULL DEFAULT 'active'
 CHECK(disposition IN ('active','needs_review','rejudge','excluded'));
ALTER TABLE candidates ADD COLUMN disposition_reason TEXT;
ALTER TABLE stages ADD COLUMN comparable INTEGER NOT NULL DEFAULT 0 CHECK(comparable>=0);
ALTER TABLE stages ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0 CHECK(excluded>=0);
ALTER TABLE stages ADD COLUMN unresolved INTEGER NOT NULL DEFAULT 0 CHECK(unresolved>=0);
ALTER TABLE attempts ADD COLUMN semantic_request_hash TEXT;
CREATE TABLE candidate_decisions (
 stage_id TEXT NOT NULL REFERENCES stages(id), id TEXT NOT NULL,
 ordinal INTEGER NOT NULL, request_json TEXT NOT NULL CHECK(json_valid(request_json)),
 previous TEXT NOT NULL, disposition TEXT NOT NULL, created_at TEXT NOT NULL,
 PRIMARY KEY(stage_id,id), FOREIGN KEY(stage_id,ordinal) REFERENCES candidates(stage_id,ordinal)
);
CREATE TABLE writer_health (id INTEGER PRIMARY KEY CHECK(id=1), probe INTEGER NOT NULL);
INSERT INTO writer_health VALUES (1,0);

-- Legacy blocked flags also mean transport/parse failures. Only accepted
-- abstentions or unknown ratings become candidate review decisions.
UPDATE candidates SET disposition='needs_review',disposition_reason='rating_unresolved'
 WHERE rating NOT IN ('g','s','q','e');
CREATE TEMP TABLE r1_abstentions (
 stage_id TEXT NOT NULL, ordinal INTEGER NOT NULL, sequence INTEGER NOT NULL, reason TEXT,
 PRIMARY KEY(stage_id,ordinal)
) WITHOUT ROWID;
INSERT INTO r1_abstentions(stage_id,ordinal,sequence,reason)
 SELECT b.stage_id,json_extract(m.value,'$.candidate.ordinal'),b.sequence,json_extract(u.value,'$.reason')
 FROM batches b,json_each(b.observation,'$.unjudgeable') u,json_each(b.members) m
 WHERE b.state='accepted' AND json_extract(m.value,'$.label')=json_extract(u.value,'$.id')
 ON CONFLICT(stage_id,ordinal) DO UPDATE SET sequence=excluded.sequence,reason=excluded.reason
 WHERE excluded.sequence>r1_abstentions.sequence;
UPDATE candidates SET disposition='needs_review',disposition_reason=COALESCE((
 SELECT reason FROM r1_abstentions a WHERE a.stage_id=candidates.stage_id AND a.ordinal=candidates.ordinal),'legacy_unjudgeable')
 WHERE blocked=1 AND EXISTS(SELECT 1 FROM r1_abstentions a WHERE a.stage_id=candidates.stage_id AND a.ordinal=candidates.ordinal);
DROP TABLE temp.r1_abstentions;

UPDATE stages SET
 comparable=(SELECT count(*) FROM candidates c WHERE c.stage_id=stages.id AND c.exposures>0 AND c.rating IN ('g','s','q','e')),
 unresolved=(SELECT count(*) FROM candidates c WHERE c.stage_id=stages.id AND
 (c.blocked=1 OR c.disposition!='active' OR c.exposures<json_extract(stages.config,'$.request.exposures')));

-- O(1) count maintenance per changed candidate, rather than re-scanning the
-- million-row stage on every accepted batch or metrics request.
CREATE TRIGGER candidate_counts_insert AFTER INSERT ON candidates BEGIN
 UPDATE stages SET
 comparable=comparable+(NEW.exposures>0 AND NEW.disposition!='excluded' AND NEW.rating IN ('g','s','q','e')),
 excluded=excluded+(NEW.disposition='excluded'),
 unresolved=unresolved+(NEW.disposition!='excluded' AND
   (NEW.blocked=1 OR NEW.disposition!='active' OR NEW.exposures<json_extract(config,'$.request.exposures')))
 WHERE id=NEW.stage_id;
END;
CREATE TRIGGER candidate_counts_update AFTER UPDATE OF exposures,blocked,disposition ON candidates BEGIN
 UPDATE stages SET
 comparable=comparable+(NEW.exposures>0 AND NEW.disposition!='excluded' AND NEW.rating IN ('g','s','q','e'))
  -(OLD.exposures>0 AND OLD.disposition!='excluded' AND OLD.rating IN ('g','s','q','e')),
 excluded=excluded+(NEW.disposition='excluded')-(OLD.disposition='excluded'),
 unresolved=unresolved+(NEW.disposition!='excluded' AND
   (NEW.blocked=1 OR NEW.disposition!='active' OR NEW.exposures<json_extract(config,'$.request.exposures')))
  -(OLD.disposition!='excluded' AND
   (OLD.blocked=1 OR OLD.disposition!='active' OR OLD.exposures<json_extract(config,'$.request.exposures')))
 WHERE id=NEW.stage_id;
END;
CREATE INDEX candidate_disposition ON candidates(stage_id,disposition,rating,reserved,exposures,sort_key);
CREATE TRIGGER stage_state_insert BEFORE INSERT ON stages WHEN
 NEW.state NOT IN ('preparing','ready','running','pausing','paused','cancelling','cancelled','needs_attention','failed','completed','completed_with_exclusions')
 OR NEW.total<1 OR NEW.frozen<0 OR NEW.frozen>NEW.total OR NEW.eligible<0 OR NEW.eligible>NEW.frozen
 OR NEW.attempts<0 OR NEW.accepted<0 OR NEW.invalid<0 OR NEW.unknown<0 OR NEW.protected<0
 BEGIN SELECT RAISE(ABORT,'invalid evaluation state or count'); END;
CREATE TRIGGER stage_state_update BEFORE UPDATE ON stages WHEN
 NEW.state NOT IN ('preparing','ready','running','pausing','paused','cancelling','cancelled','needs_attention','failed','completed','completed_with_exclusions')
 OR NEW.total<1 OR NEW.frozen<0 OR NEW.frozen>NEW.total OR NEW.eligible<0 OR NEW.eligible>NEW.frozen
 OR NEW.attempts<0 OR NEW.accepted<0 OR NEW.invalid<0 OR NEW.unknown<0 OR NEW.protected<0
 BEGIN SELECT RAISE(ABORT,'invalid evaluation state or count'); END;
PRAGMA user_version=3;

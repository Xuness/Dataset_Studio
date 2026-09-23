-- Interactive exception queues seek directly within one disposition.
CREATE INDEX candidate_disposition_page ON candidates(stage_id, disposition, ordinal);
PRAGMA user_version=4;

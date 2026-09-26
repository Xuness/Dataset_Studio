ALTER TABLE query_results ADD COLUMN storage_kind TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE query_results ADD COLUMN view_expires_ms INTEGER;
CREATE INDEX query_result_storage ON query_results(storage_kind,status);
ALTER TABLE collection_members RENAME TO collection_member_legacy;
CREATE VIEW collection_members AS SELECT * FROM collection_member_legacy;
CREATE TRIGGER collection_member_insert INSTEAD OF INSERT ON collection_members BEGIN
  INSERT OR IGNORE INTO collection_member_legacy VALUES(NEW.collection_id,NEW.source_id,NEW.asset_id);
END;
CREATE TRIGGER collection_member_delete INSTEAD OF DELETE ON collection_members BEGIN
  DELETE FROM collection_member_legacy WHERE collection_id=OLD.collection_id AND source_id=OLD.source_id AND asset_id=OLD.asset_id;
END;
CREATE TABLE collection_bases(collection_id TEXT PRIMARY KEY REFERENCES collections(id),result_id TEXT NOT NULL REFERENCES query_results(id));
CREATE TABLE collection_inclusions(collection_id TEXT NOT NULL REFERENCES collections(id),source_id TEXT NOT NULL,asset_id TEXT NOT NULL,PRIMARY KEY(collection_id,source_id,asset_id)) WITHOUT ROWID;
CREATE TABLE collection_exclusions(collection_id TEXT NOT NULL REFERENCES collections(id),source_id TEXT NOT NULL,asset_id TEXT NOT NULL,PRIMARY KEY(collection_id,source_id,asset_id)) WITHOUT ROWID;
ALTER TABLE job_inputs RENAME TO job_input_legacy;
CREATE VIEW job_inputs AS SELECT * FROM job_input_legacy;
CREATE TRIGGER job_input_insert INSTEAD OF INSERT ON job_inputs BEGIN
  INSERT OR IGNORE INTO job_input_legacy VALUES(NEW.job_id,NEW.source_id,NEW.asset_id);
END;
CREATE TRIGGER job_input_delete INSTEAD OF DELETE ON job_inputs BEGIN
  DELETE FROM job_input_legacy WHERE job_id=OLD.job_id AND source_id=OLD.source_id AND asset_id=OLD.asset_id;
END;
CREATE TABLE job_input_bases(job_id TEXT PRIMARY KEY REFERENCES jobs(id),result_id TEXT NOT NULL REFERENCES query_results(id));
CREATE TABLE job_input_exclusions(job_id TEXT NOT NULL REFERENCES jobs(id),source_id TEXT NOT NULL,asset_id TEXT NOT NULL,PRIMARY KEY(job_id,source_id,asset_id)) WITHOUT ROWID;

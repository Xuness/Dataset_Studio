-- v21: independently count section identities and retain the best expansion path.
ALTER TABLE pinterest_streams ADD COLUMN unique_sections INTEGER NOT NULL DEFAULT 0;
CREATE TABLE pinterest_stream_sections(scan_id TEXT NOT NULL REFERENCES pinterest_streams(scan_id),
 section_id TEXT NOT NULL,PRIMARY KEY(scan_id,section_id)) WITHOUT ROWID;
INSERT INTO pinterest_stream_sections
 SELECT s.scan_id,t.pin_id FROM pinterest_tasks t JOIN pinterest_streams s
 ON s.job_id=t.job_id AND s.entrypoint='board_sections'
 AND s.subject_id=json_extract(t.input_json,'$.board_id')
 AND s.root_json=json_extract(t.input_json,'$.root')
 WHERE t.kind='board_admit' AND json_extract(t.input_json,'$.source_kind')='section'
 GROUP BY s.scan_id,t.pin_id;
UPDATE pinterest_streams SET unique_sections=(SELECT count(*) FROM pinterest_stream_sections s WHERE s.scan_id=pinterest_streams.scan_id);
UPDATE pinterest_streams SET state='needs_review',reason='discovery_count_shortfall'
 WHERE entrypoint='board_sections' AND state='exhausted' AND reported_total>unique_sections;
UPDATE pinterest_jobs SET state='completed_with_gaps'
 WHERE state='completed' AND EXISTS(SELECT 1 FROM pinterest_streams s WHERE s.job_id=pinterest_jobs.id
 AND s.entrypoint='board_sections' AND s.reason='discovery_count_shortfall');
CREATE TRIGGER pinterest_stream_section_insert AFTER INSERT ON pinterest_stream_sections BEGIN
 UPDATE pinterest_streams SET unique_sections=unique_sections+1 WHERE scan_id=new.scan_id;
END;
CREATE TABLE pinterest_expansion_depths(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),root_json TEXT NOT NULL,
 kind TEXT NOT NULL,source_id TEXT NOT NULL,min_depth INTEGER NOT NULL,
 PRIMARY KEY(job_id,root_json,kind,source_id)) WITHOUT ROWID;
-- Old admission deduplication could suppress a shallower path. Do not assert it was
-- expanded: the next observed occurrence repairs that path via an accepted receipt.
CREATE TABLE pinterest_download_cleanup(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),download_key TEXT NOT NULL,
 normalized_url TEXT NOT NULL,context_id TEXT NOT NULL,generation INTEGER NOT NULL,
 PRIMARY KEY(job_id,download_key)) WITHOUT ROWID;

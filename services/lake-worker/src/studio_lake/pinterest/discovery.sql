-- v16: independent Pinterest discovery queues. Existing task identities/claims survive unchanged.
DROP TRIGGER pinterest_task_insert;
DROP TRIGGER pinterest_task_state;
DROP INDEX pinterest_tasks_queue;
ALTER TABLE pinterest_tasks RENAME TO pinterest_tasks_v15;
CREATE TABLE pinterest_tasks(task_row INTEGER PRIMARY KEY,task_id TEXT NOT NULL UNIQUE,
 job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),kind TEXT NOT NULL CHECK(kind IN
 ('pin_detail','pin_enrichment','pin_admit','media_download','board_resolve','section_resolve',
  'board_admit','board_page','section_page','board_sections','board_more_ideas','related_pins','search_page','topic_page')),
 pin_id TEXT NOT NULL,input_json TEXT NOT NULL CHECK(json_valid(input_json)),state TEXT NOT NULL,
 claim_token TEXT,receipt_id TEXT,attempts INTEGER NOT NULL DEFAULT 0,download_generation INTEGER NOT NULL DEFAULT 0,
 reason TEXT,retry_at REAL NOT NULL DEFAULT 0,updated_at TEXT NOT NULL);
INSERT INTO pinterest_tasks SELECT * FROM pinterest_tasks_v15;
DROP TABLE pinterest_tasks_v15;
CREATE INDEX pinterest_tasks_queue ON pinterest_tasks(job_id,state,retry_at,task_row);
CREATE INDEX pinterest_tasks_kind ON pinterest_tasks(job_id,kind,state,task_row);
CREATE INDEX pinterest_tasks_scan ON pinterest_tasks(job_id,json_extract(input_json,'$.scan_id'),kind);
CREATE TRIGGER pinterest_task_insert AFTER INSERT ON pinterest_tasks BEGIN
 INSERT INTO pinterest_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1;
END;
CREATE TRIGGER pinterest_task_state AFTER UPDATE OF state ON pinterest_tasks WHEN old.state<>new.state BEGIN
 UPDATE pinterest_counts SET n=n-1 WHERE job_id=old.job_id AND kind=old.kind AND state=old.state;
 INSERT INTO pinterest_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1;
END;
ALTER TABLE pinterest_jobs ADD COLUMN detail_requests INTEGER NOT NULL DEFAULT 0;
ALTER TABLE pinterest_jobs ADD COLUMN budget_round INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pinterest_jobs ADD COLUMN budget_baseline_json TEXT NOT NULL DEFAULT '{}';
UPDATE pinterest_jobs SET detail_requests=api_requests;
CREATE TABLE pinterest_streams(scan_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),
 entrypoint TEXT NOT NULL,subject_id TEXT NOT NULL,root_json TEXT NOT NULL CHECK(json_valid(root_json)),
 parameters_json TEXT NOT NULL CHECK(json_valid(parameters_json)),depth INTEGER NOT NULL,
 state TEXT NOT NULL,cursor_json TEXT,reason TEXT,pages INTEGER NOT NULL DEFAULT 0,
 members INTEGER NOT NULL DEFAULT 0,force_detail INTEGER NOT NULL DEFAULT 0,
 samples_checked INTEGER NOT NULL DEFAULT 0,mismatches INTEGER NOT NULL DEFAULT 0,
 last_turn INTEGER NOT NULL DEFAULT 0,updated_at TEXT NOT NULL);
CREATE INDEX pinterest_stream_job ON pinterest_streams(job_id,state,scan_id);
CREATE TABLE pinterest_stream_pages(scan_id TEXT NOT NULL REFERENCES pinterest_streams(scan_id),
 page_key TEXT NOT NULL,receipt_id TEXT NOT NULL,PRIMARY KEY(scan_id,page_key)) WITHOUT ROWID;
CREATE TABLE pinterest_admitted(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),kind TEXT NOT NULL,
 source_id TEXT NOT NULL,receipt_id TEXT NOT NULL,PRIMARY KEY(job_id,kind,source_id)) WITHOUT ROWID;
CREATE TABLE pinterest_metrics(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),name TEXT NOT NULL,
 value INTEGER NOT NULL,PRIMARY KEY(job_id,name)) WITHOUT ROWID;

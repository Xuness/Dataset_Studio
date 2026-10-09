CREATE TABLE pinterest_lakes(lake_id TEXT PRIMARY KEY REFERENCES lakes(id),archive_format INTEGER NOT NULL CHECK(archive_format=3),
 online_format INTEGER NOT NULL CHECK(online_format=4));
CREATE TABLE pinterest_requests(request_key TEXT PRIMARY KEY,operation TEXT NOT NULL,request_hash TEXT NOT NULL,
 subject_id TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('pending','succeeded')),result_json TEXT,created_at TEXT NOT NULL);
CREATE TABLE pinterest_jobs(job_row INTEGER PRIMARY KEY,id TEXT NOT NULL UNIQUE,request_key TEXT NOT NULL UNIQUE,
 lake_id TEXT NOT NULL REFERENCES pinterest_lakes(lake_id),definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 definition_sha256 TEXT NOT NULL,context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 state TEXT NOT NULL,desired_state TEXT NOT NULL CHECK(desired_state IN ('running','paused','cancelled')),
 revision INTEGER NOT NULL DEFAULT 0,execution_epoch INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,
 retry_at REAL NOT NULL DEFAULT 0,error_code TEXT,error_message TEXT,api_requests INTEGER NOT NULL DEFAULT 0,
 download_bytes INTEGER NOT NULL DEFAULT 0,elapsed_seconds REAL NOT NULL DEFAULT 0,
 archive_seq INTEGER NOT NULL DEFAULT 0,served_seq INTEGER NOT NULL DEFAULT 0,cleanup_state TEXT NOT NULL DEFAULT 'pending');
CREATE INDEX pinterest_jobs_lake ON pinterest_jobs(lake_id,state,retry_at,created_at,id);
CREATE INDEX pinterest_jobs_recent ON pinterest_jobs(created_at DESC,id DESC);
CREATE TABLE pinterest_tasks(task_row INTEGER PRIMARY KEY,task_id TEXT NOT NULL UNIQUE,
 job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),kind TEXT NOT NULL CHECK(kind IN ('pin_detail','media_download')),
 pin_id TEXT NOT NULL,input_json TEXT NOT NULL CHECK(json_valid(input_json)),state TEXT NOT NULL,
 claim_token TEXT,receipt_id TEXT,attempts INTEGER NOT NULL DEFAULT 0,download_generation INTEGER NOT NULL DEFAULT 0,
 reason TEXT,retry_at REAL NOT NULL DEFAULT 0,updated_at TEXT NOT NULL);
CREATE INDEX pinterest_tasks_queue ON pinterest_tasks(job_id,state,retry_at,task_row);
CREATE TABLE pinterest_counts(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),kind TEXT NOT NULL,state TEXT NOT NULL,n INTEGER NOT NULL,
 PRIMARY KEY(job_id,kind,state)) WITHOUT ROWID;
CREATE TRIGGER pinterest_task_insert AFTER INSERT ON pinterest_tasks BEGIN
 INSERT INTO pinterest_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1;
END;
CREATE TRIGGER pinterest_task_state AFTER UPDATE OF state ON pinterest_tasks WHEN old.state<>new.state BEGIN
 UPDATE pinterest_counts SET n=n-1 WHERE job_id=old.job_id AND kind=old.kind AND state=old.state;
 INSERT INTO pinterest_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1;
END;
CREATE TABLE pinterest_applied(receipt_id TEXT PRIMARY KEY,job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),seq INTEGER NOT NULL);
CREATE TABLE pinterest_downloads(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),download_key TEXT NOT NULL,
 acquisition_json TEXT NOT NULL CHECK(json_valid(acquisition_json)),PRIMARY KEY(job_id,download_key)) WITHOUT ROWID;

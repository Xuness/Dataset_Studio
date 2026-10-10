-- v19: repair derived state omitted by early development builds already marked v18.
-- State holds the runner/execution locks and backs up before this transaction.
CREATE TABLE IF NOT EXISTS pinterest_admitted_counts(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),
 kind TEXT NOT NULL,n INTEGER NOT NULL,PRIMARY KEY(job_id,kind)) WITHOUT ROWID;
DELETE FROM pinterest_admitted_counts;
INSERT INTO pinterest_admitted_counts SELECT job_id,kind,count(*) FROM pinterest_admitted GROUP BY job_id,kind;
DROP TRIGGER IF EXISTS pinterest_admitted_insert;
CREATE TRIGGER pinterest_admitted_insert AFTER INSERT ON pinterest_admitted BEGIN
 INSERT INTO pinterest_admitted_counts VALUES(new.job_id,new.kind,1) ON CONFLICT(job_id,kind) DO UPDATE SET n=n+1;
END;
CREATE INDEX IF NOT EXISTS pinterest_jobs_state ON pinterest_jobs(state,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS pinterest_tasks_scan ON pinterest_tasks(job_id,json_extract(input_json,'$.scan_id'),kind);

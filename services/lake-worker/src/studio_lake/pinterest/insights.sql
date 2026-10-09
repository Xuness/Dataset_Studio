CREATE TABLE pinterest_seen(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),entrypoint TEXT NOT NULL,
 kind TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(job_id,entrypoint,kind,value)) WITHOUT ROWID;

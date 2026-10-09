CREATE TABLE pinterest_reuse(lake_id TEXT NOT NULL REFERENCES pinterest_lakes(lake_id),reuse_key TEXT NOT NULL,
 value_json TEXT NOT NULL CHECK(json_valid(value_json)),PRIMARY KEY(lake_id,reuse_key)) WITHOUT ROWID;
INSERT INTO pinterest_admitted(job_id,kind,source_id,receipt_id)
 SELECT job_id,'pin',pin_id,coalesce(receipt_id,'legacy-v15') FROM pinterest_tasks WHERE kind='pin_detail' AND attempts>0
 ON CONFLICT DO NOTHING;
CREATE TABLE pinterest_admitted_counts(job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),kind TEXT NOT NULL,n INTEGER NOT NULL,
 PRIMARY KEY(job_id,kind)) WITHOUT ROWID;
INSERT INTO pinterest_admitted_counts SELECT job_id,kind,count(*) FROM pinterest_admitted GROUP BY job_id,kind;
CREATE TRIGGER pinterest_admitted_insert AFTER INSERT ON pinterest_admitted BEGIN
 INSERT INTO pinterest_admitted_counts VALUES(new.job_id,new.kind,1) ON CONFLICT(job_id,kind) DO UPDATE SET n=n+1;
END;
CREATE TABLE pinterest_schedules(id TEXT PRIMARY KEY,definition_json TEXT NOT NULL CHECK(json_valid(definition_json)),
 lake_id TEXT NOT NULL REFERENCES pinterest_lakes(lake_id),every_seconds INTEGER NOT NULL,next_at REAL NOT NULL,
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),revision INTEGER NOT NULL,last_job TEXT,
 created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
CREATE INDEX pinterest_schedules_due ON pinterest_schedules(enabled,next_at,id);
CREATE INDEX pinterest_schedules_lake ON pinterest_schedules(lake_id,id);
CREATE TABLE pinterest_schedule_runs(schedule_id TEXT NOT NULL,revision INTEGER NOT NULL,occurrence REAL NOT NULL,
 job_id TEXT NOT NULL REFERENCES pinterest_jobs(id),PRIMARY KEY(schedule_id,revision,occurrence)) WITHOUT ROWID;
CREATE INDEX pinterest_schedule_job ON pinterest_schedule_runs(job_id);

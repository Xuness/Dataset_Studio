CREATE TABLE collection_lakes(
  lake_id TEXT PRIMARY KEY REFERENCES lakes(id),
  archive_format INTEGER NOT NULL CHECK(archive_format=2),
  online_format INTEGER NOT NULL CHECK(online_format=3),
  collector TEXT NOT NULL CHECK(collector='pixiv_web_v1')
);
CREATE TABLE collection_accounts(
  id TEXT PRIMARY KEY, site TEXT NOT NULL CHECK(site='pixiv'), label TEXT NOT NULL,
  mode TEXT NOT NULL CHECK(mode IN ('anonymous','session')),
  viewer_key TEXT NOT NULL UNIQUE, bound_user_id TEXT,
  secret_blob BLOB, revision INTEGER NOT NULL CHECK(revision > 0),
  state TEXT NOT NULL CHECK(state IN ('unverified','valid','expired','challenge','cleared')),
  last_probe_json TEXT CHECK(last_probe_json IS NULL OR json_valid(last_probe_json)),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE collection_requests(
  request_key TEXT PRIMARY KEY, operation TEXT NOT NULL, request_hash TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('pending','succeeded','failed')),
  subject_id TEXT, result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE collection_jobs(
  job_row INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
  request_key TEXT NOT NULL UNIQUE REFERENCES collection_requests(request_key),
  lake_id TEXT NOT NULL REFERENCES collection_lakes(lake_id),
  account_id TEXT NOT NULL REFERENCES collection_accounts(id),
  definition_json TEXT NOT NULL CHECK(json_valid(definition_json)), definition_sha256 TEXT NOT NULL,
  desired_state TEXT NOT NULL CHECK(desired_state IN ('run','pause','cancel')),
  state TEXT NOT NULL CHECK(state IN ('queued','running','pausing','paused','waiting_credentials','waiting_retry',
    'waiting_resources','waiting_budget','publishing','cancelling','cancelled','completed','completed_with_gaps','needs_review')),
  revision INTEGER NOT NULL CHECK(revision > 0), execution_epoch INTEGER NOT NULL DEFAULT 0 CHECK(execution_epoch >= 0),
  retry_at_ms INTEGER NOT NULL DEFAULT 0, visibility_json TEXT CHECK(visibility_json IS NULL OR json_valid(visibility_json)),
  counters_json TEXT NOT NULL CHECK(json_valid(counters_json)),
  error_code TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX collection_job_due ON collection_jobs(lake_id,state,retry_at_ms,job_row);
CREATE INDEX collection_job_state ON collection_jobs(state,job_row);
CREATE TABLE collection_entities(
  job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  kind TEXT NOT NULL CHECK(kind IN ('author','work')), source_id TEXT NOT NULL,
  min_depth INTEGER NOT NULL CHECK(min_depth >= 0),
  expanded_depth INTEGER CHECK(expanded_depth >= 0),
  state TEXT NOT NULL CHECK(state IN ('candidate','admitted','processed','gap','excluded')),
  provenance_count INTEGER NOT NULL DEFAULT 0, first_task_id TEXT,
  PRIMARY KEY(job_id,kind,source_id)
) WITHOUT ROWID;
CREATE INDEX collection_frontier ON collection_entities(job_id,state,kind,min_depth,source_id);
CREATE TABLE collection_tasks(
  task_row INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
  job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  kind TEXT NOT NULL CHECK(kind IN ('author_profile','author_directory','work_detail','media_manifest',
    'relationship_page','media_download','media_encode','media_poster')),
  subject_key TEXT NOT NULL, task_key TEXT NOT NULL, payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
  state TEXT NOT NULL CHECK(state IN ('blocked','queued','running','staged','archived','done','retry_wait',
    'waiting_credentials','waiting_resources','unavailable','excluded','needs_review','cancelled')),
  priority INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
  retry_at_ms INTEGER NOT NULL DEFAULT 0, remaining_dependencies INTEGER NOT NULL DEFAULT 0 CHECK(remaining_dependencies >= 0),
  claimed_epoch INTEGER, claim_token TEXT, result_receipt TEXT, reason TEXT,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  UNIQUE(job_id,task_key), UNIQUE(id,job_id),
  CHECK(state NOT IN ('queued','running','staged','archived','done') OR remaining_dependencies=0),
  CHECK(state <> 'running' OR (claimed_epoch IS NOT NULL AND claim_token IS NOT NULL))
);
CREATE INDEX collection_task_due ON collection_tasks(job_id,state,retry_at_ms,priority,task_row);
CREATE INDEX collection_task_subject ON collection_tasks(job_id,kind,subject_key,task_row);
CREATE INDEX collection_task_kind ON collection_tasks(job_id,kind,task_row);
CREATE INDEX collection_task_reason ON collection_tasks(job_id,reason,task_row);
CREATE TABLE collection_dependencies(
  job_id TEXT NOT NULL, parent_task_id TEXT NOT NULL, child_task_id TEXT NOT NULL,
  required_stage TEXT NOT NULL CHECK(required_stage IN ('staged','archived','done')),
  satisfied INTEGER NOT NULL DEFAULT 0 CHECK(satisfied IN (0,1)),
  PRIMARY KEY(parent_task_id,child_task_id),
  FOREIGN KEY(parent_task_id,job_id) REFERENCES collection_tasks(id,job_id),
  FOREIGN KEY(child_task_id,job_id) REFERENCES collection_tasks(id,job_id),
  CHECK(parent_task_id <> child_task_id)
) WITHOUT ROWID;
CREATE INDEX collection_dependencies_child ON collection_dependencies(child_task_id,satisfied);
CREATE TRIGGER collection_dependency_order BEFORE INSERT ON collection_dependencies
WHEN (SELECT task_row FROM collection_tasks WHERE id=NEW.parent_task_id)
  >= (SELECT task_row FROM collection_tasks WHERE id=NEW.child_task_id)
BEGIN SELECT RAISE(ABORT,'collection dependency must point to a newly created child'); END;
CREATE TABLE collection_checkpoints(
  job_id TEXT NOT NULL REFERENCES collection_jobs(id), stream_key TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK(revision >= 0), cursor_json TEXT NOT NULL CHECK(json_valid(cursor_json)),
  archive_seq INTEGER NOT NULL, batch_id TEXT NOT NULL,
  PRIMARY KEY(job_id,stream_key)
) WITHOUT ROWID;
CREATE TABLE collection_outbox(
  id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  task_id TEXT, execution_epoch INTEGER NOT NULL, claim_token TEXT,
  dedupe_key TEXT NOT NULL UNIQUE, intent_path TEXT NOT NULL, content_sha256 TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('prepared','archive_committed','released','needs_review')),
  archive_seq INTEGER, batch_id TEXT,
  control_applied INTEGER NOT NULL DEFAULT 0 CHECK(control_applied IN (0,1)),
  published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1)),
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  FOREIGN KEY(task_id,job_id) REFERENCES collection_tasks(id,job_id),
  CHECK((control_applied=0 AND published=0) OR (archive_seq IS NOT NULL AND batch_id IS NOT NULL)),
  CHECK(state <> 'released' OR (control_applied=1 AND published=1))
) WITHOUT ROWID;
CREATE INDEX collection_outbox_pending ON collection_outbox(job_id,state,id);
CREATE TABLE collection_applied_batches(
  lake_id TEXT NOT NULL REFERENCES collection_lakes(lake_id), seq INTEGER NOT NULL,
  batch_id TEXT NOT NULL, job_id TEXT NOT NULL REFERENCES collection_jobs(id),
  receipt_sha256 TEXT NOT NULL, applied_at TEXT NOT NULL,
  PRIMARY KEY(lake_id,seq), UNIQUE(lake_id,batch_id)
) WITHOUT ROWID;
CREATE TABLE collection_lake_dispatch(
  lake_id TEXT PRIMARY KEY REFERENCES lakes(id), last_family TEXT NOT NULL CHECK(last_family IN ('update','collection')),
  last_job_id TEXT, turn INTEGER NOT NULL DEFAULT 0
);
-- Rebuildable control indexes, populated only from archived plans and receipts.
CREATE TABLE collection_counts(job_id TEXT NOT NULL REFERENCES collection_jobs(id),kind TEXT NOT NULL,state TEXT NOT NULL,n INTEGER NOT NULL,PRIMARY KEY(job_id,kind,state)) WITHOUT ROWID;
CREATE TRIGGER collection_count_insert AFTER INSERT ON collection_tasks BEGIN
  INSERT INTO collection_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1; END;
CREATE TRIGGER collection_count_update AFTER UPDATE OF state ON collection_tasks WHEN old.state<>new.state BEGIN
  UPDATE collection_counts SET n=n-1 WHERE job_id=old.job_id AND kind=old.kind AND state=old.state;
  INSERT INTO collection_counts VALUES(new.job_id,new.kind,new.state,1) ON CONFLICT(job_id,kind,state) DO UPDATE SET n=n+1; END;
CREATE TABLE collection_discovery_edges(
  job_id TEXT NOT NULL REFERENCES collection_jobs(id),source_kind TEXT NOT NULL,source_id TEXT NOT NULL,
  target_kind TEXT NOT NULL,target_id TEXT NOT NULL,depth_step INTEGER NOT NULL CHECK(depth_step IN (0,1)),snapshot_id TEXT NOT NULL,
  PRIMARY KEY(job_id,source_kind,source_id,target_kind,target_id,snapshot_id)
) WITHOUT ROWID;
CREATE INDEX collection_edge_target ON collection_discovery_edges(job_id,target_kind,target_id);

-- Design contract canonical-media-v2 / online schema 3, not a production migration.
-- Execute only against a new empty database. The publisher validates immutable rows
-- before INSERT; a duplicate ID with different canonical contents is an error.
PRAGMA foreign_keys = ON;
PRAGMA application_id = 1146310482;
PRAGMA user_version = 3;

CREATE TABLE online_state(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE publications(
  seq INTEGER PRIMARY KEY CHECK(seq > 0), batch_id TEXT NOT NULL UNIQUE,
  manifest_sha256 TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('preparing','published')),
  archived_at TEXT NOT NULL, published_at TEXT, counts_json TEXT NOT NULL CHECK(json_valid(counts_json))
);
CREATE TABLE visibility_contexts(
  context_id TEXT PRIMARY KEY, viewer_key TEXT NOT NULL,
  policy_json TEXT NOT NULL CHECK(json_valid(policy_json)), comparison_key TEXT,
  verified INTEGER NOT NULL CHECK(verified IN (0,1)), observed_at TEXT NOT NULL,
  commit_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE TABLE captures(
  capture_id TEXT PRIMARY KEY, request_receipt_id TEXT NOT NULL UNIQUE, endpoint TEXT NOT NULL,
  subject_kind TEXT NOT NULL CHECK(subject_kind IN ('author','work','directory','relation','media','probe')),
  subject_id TEXT NOT NULL, request_json TEXT NOT NULL CHECK(json_valid(request_json)),
  context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),
  observed_at TEXT NOT NULL, http_status INTEGER NOT NULL CHECK(http_status BETWEEN 100 AND 599),
  source_error TEXT, adapter_version TEXT NOT NULL,
  raw_format TEXT NOT NULL CHECK(raw_format IN ('json','text','binary')),
  raw_bytes INTEGER NOT NULL CHECK(raw_bytes >= 0), raw_sha256 TEXT NOT NULL,
  raw_zlib BLOB NOT NULL, commit_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE INDEX captures_subject ON captures(subject_kind,subject_id,observed_at,capture_id);
CREATE TABLE authors(
  author_id TEXT PRIMARY KEY, first_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE TABLE author_observations(
  observation_id TEXT PRIMARY KEY, author_id TEXT NOT NULL REFERENCES authors(author_id),
  capture_id TEXT NOT NULL REFERENCES captures(capture_id), observed_at TEXT NOT NULL,
  normalizer_version TEXT NOT NULL,
  display_name TEXT, profile_json TEXT NOT NULL CHECK(json_valid(profile_json)),
  commit_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE INDEX author_history ON author_observations(author_id,observed_at,observation_id);
CREATE TABLE works(
  work_id TEXT PRIMARY KEY, first_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE TABLE work_observations(
  row_id INTEGER PRIMARY KEY, observation_id TEXT NOT NULL UNIQUE,
  work_id TEXT NOT NULL REFERENCES works(work_id), capture_id TEXT NOT NULL REFERENCES captures(capture_id),
  observed_at TEXT NOT NULL, normalizer_version TEXT NOT NULL, author_id TEXT REFERENCES authors(author_id),
  work_type TEXT NOT NULL CHECK(work_type IN ('illustration','manga','ugoira','unknown')),
  title TEXT, caption_html TEXT,
  page_count INTEGER CHECK(page_count >= 0), created_at TEXT, updated_at TEXT,
  source_fields_json TEXT NOT NULL CHECK(json_valid(source_fields_json)),
  issues_json TEXT NOT NULL CHECK(json_valid(issues_json)),
  commit_seq INTEGER NOT NULL REFERENCES publications(seq), UNIQUE(observation_id,work_id)
);
CREATE INDEX work_history ON work_observations(work_id,observed_at,observation_id);
CREATE INDEX work_author ON work_observations(author_id,work_id,observation_id);
-- Site-profile indexes; base record identity/relations do not depend on Pixiv fields.
CREATE INDEX pixiv_rating ON work_observations(json_extract(source_fields_json,'$.pixiv.x_restrict'),work_id,observation_id);
CREATE INDEX pixiv_ai ON work_observations(json_extract(source_fields_json,'$.pixiv.ai_type'),work_id,observation_id);
CREATE INDEX pixiv_bookmarks ON work_observations(json_extract(source_fields_json,'$.pixiv.bookmark_count'),work_id,observation_id);
CREATE TABLE tags(tag_id INTEGER PRIMARY KEY, tag TEXT NOT NULL UNIQUE COLLATE BINARY);
CREATE TABLE work_tags(
  observation_id TEXT NOT NULL REFERENCES work_observations(observation_id),
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0), tag_id INTEGER NOT NULL REFERENCES tags(tag_id),
  translations_json TEXT NOT NULL CHECK(json_valid(translations_json)),
  locked INTEGER CHECK(locked IN (0,1)), PRIMARY KEY(observation_id,ordinal)
) WITHOUT ROWID;
CREATE INDEX tag_works ON work_tags(tag_id,observation_id);
CREATE VIRTUAL TABLE tag_index USING fts5(tokens,content='',contentless_delete=1,detail='none',tokenize='ascii');

CREATE TABLE media_manifests(
  manifest_id TEXT PRIMARY KEY, work_id TEXT NOT NULL REFERENCES works(work_id),
  capture_id TEXT NOT NULL REFERENCES captures(capture_id),
  detail_observation_id TEXT, context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),
  observed_at TEXT NOT NULL, normalizer_version TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('image_pages','ugoira')),
  expected_count INTEGER CHECK(expected_count >= 0), item_count INTEGER NOT NULL CHECK(item_count >= 0),
  complete INTEGER NOT NULL CHECK(complete IN (0,1)),
  reason TEXT, commit_seq INTEGER NOT NULL REFERENCES publications(seq),
  UNIQUE(manifest_id,work_id),
  FOREIGN KEY(detail_observation_id,work_id) REFERENCES work_observations(observation_id,work_id),
  CHECK(complete=0 OR expected_count IS NULL OR expected_count=item_count)
) WITHOUT ROWID;
CREATE INDEX work_manifests ON media_manifests(work_id,observed_at,manifest_id);
CREATE TABLE media_entries(
  media_id TEXT PRIMARY KEY, manifest_id TEXT NOT NULL,
  work_id TEXT NOT NULL, slot_key TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  kind TEXT NOT NULL CHECK(kind IN ('image','ugoira')),
  width INTEGER CHECK(width > 0), height INTEGER CHECK(height > 0),
  source_url TEXT NOT NULL, source_variant TEXT NOT NULL,
  auxiliary_urls_json TEXT NOT NULL CHECK(json_valid(auxiliary_urls_json)),
  commit_seq INTEGER NOT NULL REFERENCES publications(seq),
  FOREIGN KEY(manifest_id,work_id) REFERENCES media_manifests(manifest_id,work_id),
  UNIQUE(manifest_id,slot_key), UNIQUE(manifest_id,ordinal), UNIQUE(media_id,work_id)
) WITHOUT ROWID;
CREATE INDEX media_work ON media_entries(work_id,manifest_id,ordinal);
CREATE TABLE animation_frames(
  media_id TEXT NOT NULL REFERENCES media_entries(media_id),
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0), file_name TEXT NOT NULL,
  delay_ms INTEGER NOT NULL CHECK(delay_ms >= 0),
  PRIMARY KEY(media_id,ordinal), UNIQUE(media_id,file_name)
) WITHOUT ROWID;

CREATE TABLE objects(
  object_row INTEGER PRIMARY KEY, sha256 TEXT NOT NULL UNIQUE,
  pack_path TEXT NOT NULL, offset INTEGER NOT NULL CHECK(offset >= 0), length INTEGER NOT NULL CHECK(length >= 0),
  stored_ext TEXT NOT NULL, content_type TEXT NOT NULL,
  media_category TEXT NOT NULL CHECK(media_category IN ('image','animation','archive','other')),
  stored_width INTEGER CHECK(stored_width > 0), stored_height INTEGER CHECK(stored_height > 0),
  first_seq INTEGER NOT NULL REFERENCES publications(seq),
  CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*')
);
CREATE INDEX objects_sequence ON objects(first_seq,sha256);
CREATE TABLE assets(
  asset_id TEXT PRIMARY KEY, media_id TEXT NOT NULL REFERENCES media_entries(media_id),
  sha256 TEXT NOT NULL REFERENCES objects(sha256),
  representation TEXT NOT NULL CHECK(representation IN ('original','derived','poster')),
  recipe_id TEXT NOT NULL, acquisition_receipt_id TEXT NOT NULL,
  source_sha256 TEXT, source_bytes INTEGER CHECK(source_bytes >= 0),
  acquired_at TEXT NOT NULL, last_verified_at TEXT,
  evidence TEXT NOT NULL CHECK(evidence IN ('downloaded','http_validated','historical_reuse','derived')),
  derived_from_asset_id TEXT REFERENCES assets(asset_id),
  details_json TEXT NOT NULL CHECK(json_valid(details_json)),
  commit_seq INTEGER NOT NULL REFERENCES publications(seq),
  UNIQUE(asset_id,media_id,representation,recipe_id),
  CHECK(source_sha256 IS NULL OR (length(source_sha256)=64 AND source_sha256 NOT GLOB '*[^0-9a-f]*'))
) WITHOUT ROWID;
CREATE INDEX asset_media ON assets(media_id,representation,recipe_id,commit_seq,asset_id);
CREATE INDEX asset_object ON assets(sha256,asset_id);

CREATE TABLE discovery_snapshots(
  snapshot_id TEXT PRIMARY KEY, capture_id TEXT NOT NULL REFERENCES captures(capture_id),
  root_kind TEXT NOT NULL CHECK(root_kind IN ('author','work','query')),
  root_id TEXT NOT NULL, relation TEXT NOT NULL,
  context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id), observed_at TEXT NOT NULL,
  scan_id TEXT NOT NULL, stream_key TEXT NOT NULL, planner_version TEXT NOT NULL,
  page_key TEXT NOT NULL, next_cursor_json TEXT CHECK(next_cursor_json IS NULL OR json_valid(next_cursor_json)),
  page_complete INTEGER NOT NULL CHECK(page_complete IN (0,1)),
  traversal_exhausted INTEGER NOT NULL CHECK(traversal_exhausted IN (0,1)),
  commit_seq INTEGER NOT NULL REFERENCES publications(seq)
) WITHOUT ROWID;
CREATE INDEX discovery_root ON discovery_snapshots(root_kind,root_id,relation,scan_id,page_key);
CREATE INDEX discovery_recent ON discovery_snapshots(root_kind,root_id,relation,observed_at,snapshot_id);
CREATE TABLE discovery_members(
  snapshot_id TEXT NOT NULL REFERENCES discovery_snapshots(snapshot_id),
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0), target_kind TEXT NOT NULL CHECK(target_kind IN ('author','work')),
  target_id TEXT NOT NULL, PRIMARY KEY(snapshot_id,ordinal)
) WITHOUT ROWID;
CREATE INDEX discovery_target ON discovery_members(target_kind,target_id,snapshot_id);

-- Projection intervals are [valid_from, valid_until); NULL is unbounded.
CREATE TABLE work_versions(
  work_id TEXT NOT NULL REFERENCES works(work_id), valid_from INTEGER NOT NULL REFERENCES publications(seq),
  valid_until INTEGER REFERENCES publications(seq), observation_id TEXT, manifest_id TEXT,
  manifest_state TEXT NOT NULL CHECK(manifest_state IN ('missing','ready','needs_refresh','visibility_changed')),
  PRIMARY KEY(work_id,valid_from),
  FOREIGN KEY(observation_id,work_id) REFERENCES work_observations(observation_id,work_id),
  FOREIGN KEY(manifest_id,work_id) REFERENCES media_manifests(manifest_id,work_id),
  CHECK(valid_until IS NULL OR valid_until > valid_from)
) WITHOUT ROWID;
CREATE UNIQUE INDEX work_current ON work_versions(work_id) WHERE valid_until IS NULL;
CREATE INDEX work_version_manifest ON work_versions(manifest_id,valid_from,valid_until);
CREATE TABLE media_asset_versions(
  media_id TEXT NOT NULL, representation TEXT NOT NULL, recipe_id TEXT NOT NULL,
  valid_from INTEGER NOT NULL REFERENCES publications(seq), valid_until INTEGER REFERENCES publications(seq),
  asset_id TEXT NOT NULL,
  PRIMARY KEY(media_id,representation,recipe_id,valid_from),
  FOREIGN KEY(asset_id,media_id,representation,recipe_id) REFERENCES assets(asset_id,media_id,representation,recipe_id),
  CHECK(valid_until IS NULL OR valid_until > valid_from)
) WITHOUT ROWID;
CREATE UNIQUE INDEX media_asset_current ON media_asset_versions(media_id,representation,recipe_id) WHERE valid_until IS NULL;
CREATE TABLE changes(
  seq INTEGER NOT NULL REFERENCES publications(seq), sha256 TEXT NOT NULL REFERENCES objects(sha256),
  fields_json TEXT NOT NULL CHECK(json_valid(fields_json)), PRIMARY KEY(seq,sha256)
) WITHOUT ROWID;
CREATE TABLE leases(
  id TEXT PRIMARY KEY, seq INTEGER NOT NULL, expires_ms INTEGER, owner TEXT NOT NULL, purpose TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX lease_sequence ON leases(seq,expires_ms);
CREATE TABLE build_progress(
  name TEXT PRIMARY KEY, position TEXT NOT NULL, rows INTEGER NOT NULL, digest TEXT NOT NULL,
  complete INTEGER NOT NULL CHECK(complete IN (0,1))
) WITHOUT ROWID;
CREATE TABLE pending_publication(
  seq INTEGER PRIMARY KEY REFERENCES publications(seq), batch_id TEXT NOT NULL, fingerprint TEXT NOT NULL
);

-- Runtime readers create snapshot TEMP VIEWs using a validated numeric sequence S.
-- Every fact is filtered by commit_seq <= S (objects by first_seq <= S).
-- Serving S must never exceed online_state.served_seq. Raw tables are writer-only.

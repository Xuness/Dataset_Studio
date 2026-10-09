PRAGMA foreign_keys=ON;
PRAGMA application_id=1146310482;
PRAGMA user_version=4;
CREATE TABLE online_state(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE publications(seq INTEGER PRIMARY KEY CHECK(seq>0),batch_id TEXT NOT NULL UNIQUE,
 manifest_sha256 TEXT NOT NULL,state TEXT NOT NULL CHECK(state='published'),archived_at TEXT NOT NULL,
 published_at TEXT NOT NULL,counts_json TEXT NOT NULL CHECK(json_valid(counts_json)));
CREATE TABLE visibility_contexts(context_id TEXT PRIMARY KEY,policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
 observed_at TEXT NOT NULL,commit_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE TABLE captures(capture_id TEXT PRIMARY KEY,request_receipt_id TEXT NOT NULL UNIQUE,endpoint TEXT NOT NULL,
 subject_id TEXT NOT NULL,request_json TEXT NOT NULL CHECK(json_valid(request_json)),
 context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),observed_at TEXT NOT NULL,
 http_status INTEGER NOT NULL CHECK(http_status BETWEEN 100 AND 599),source_error TEXT,adapter_version TEXT NOT NULL,
 raw_format TEXT NOT NULL CHECK(raw_format IN ('json','text','binary')),raw_bytes INTEGER NOT NULL CHECK(raw_bytes>=0),
 raw_sha256 TEXT NOT NULL,raw_zlib BLOB NOT NULL,commit_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE INDEX captures_subject ON captures(subject_id,observed_at,capture_id);
CREATE TABLE source_entities(entity_id TEXT PRIMARY KEY,kind TEXT NOT NULL CHECK(kind IN ('board','section','account','topic','external')),
 source_id TEXT NOT NULL,first_seq INTEGER NOT NULL REFERENCES publications(seq),UNIQUE(kind,source_id)) WITHOUT ROWID;
CREATE TABLE entity_observations(observation_id TEXT PRIMARY KEY,entity_id TEXT NOT NULL REFERENCES source_entities(entity_id),
 capture_id TEXT NOT NULL REFERENCES captures(capture_id),observed_at TEXT NOT NULL,
 fields_json TEXT NOT NULL CHECK(json_valid(fields_json)),commit_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE INDEX entity_history ON entity_observations(entity_id,observed_at,observation_id);
CREATE TABLE pins(pin_id TEXT PRIMARY KEY,first_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE INDEX pins_sequence ON pins(first_seq,pin_id);
CREATE TABLE pin_observations(observation_id TEXT PRIMARY KEY,pin_id TEXT NOT NULL REFERENCES pins(pin_id),
 capture_id TEXT NOT NULL REFERENCES captures(capture_id),observed_at TEXT NOT NULL,normalizer_version TEXT NOT NULL,
 observation_kind TEXT NOT NULL CHECK(observation_kind IN ('detail','list')),field_set TEXT NOT NULL,
 title TEXT,description TEXT,image_signature TEXT,fields_json TEXT NOT NULL CHECK(json_valid(fields_json)),
 present_fields_json TEXT NOT NULL CHECK(json_valid(present_fields_json)),issues_json TEXT NOT NULL CHECK(json_valid(issues_json)),
 commit_seq INTEGER NOT NULL REFERENCES publications(seq),UNIQUE(observation_id,pin_id)) WITHOUT ROWID;
CREATE INDEX pin_history ON pin_observations(pin_id,observed_at,observation_id);
CREATE INDEX pin_history_page ON pin_observations(pin_id,observation_id);
CREATE INDEX pin_signatures ON pin_observations(image_signature,pin_id,observation_id);
CREATE TABLE media_manifests(manifest_id TEXT PRIMARY KEY,pin_id TEXT NOT NULL REFERENCES pins(pin_id),
 capture_id TEXT NOT NULL REFERENCES captures(capture_id),observation_id TEXT NOT NULL,
 context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),observed_at TEXT NOT NULL,
 normalizer_version TEXT NOT NULL,kind TEXT NOT NULL,content_revision TEXT,
 expected_count INTEGER CHECK(expected_count>=0),item_count INTEGER NOT NULL CHECK(item_count>=0),
 complete INTEGER NOT NULL CHECK(complete IN (0,1)),reason TEXT,commit_seq INTEGER NOT NULL REFERENCES publications(seq),
 UNIQUE(manifest_id,pin_id),FOREIGN KEY(observation_id,pin_id) REFERENCES pin_observations(observation_id,pin_id),
 CHECK(complete=0 OR expected_count=item_count)) WITHOUT ROWID;
CREATE INDEX pin_manifests ON media_manifests(pin_id,observed_at,manifest_id);
CREATE TABLE media_entries(media_id TEXT PRIMARY KEY,manifest_id TEXT NOT NULL,pin_id TEXT NOT NULL,
 slot_key TEXT NOT NULL,ordinal INTEGER NOT NULL CHECK(ordinal>=0),kind TEXT NOT NULL CHECK(kind='image'),
 role TEXT NOT NULL CHECK(role='original'),source_url TEXT NOT NULL,normalized_url TEXT NOT NULL,
 field_path TEXT NOT NULL,width INTEGER NOT NULL CHECK(width>0),height INTEGER NOT NULL CHECK(height>0),image_signature TEXT,
 commit_seq INTEGER NOT NULL REFERENCES publications(seq),FOREIGN KEY(manifest_id,pin_id) REFERENCES media_manifests(manifest_id,pin_id),
 UNIQUE(manifest_id,ordinal),UNIQUE(manifest_id,slot_key)) WITHOUT ROWID;
CREATE INDEX media_pin ON media_entries(pin_id,manifest_id,ordinal);
CREATE TABLE objects(object_row INTEGER PRIMARY KEY,sha256 TEXT NOT NULL UNIQUE,
 pack_path TEXT NOT NULL,offset INTEGER NOT NULL CHECK(offset>=0),length INTEGER NOT NULL CHECK(length>0),
 stored_ext TEXT NOT NULL,content_type TEXT NOT NULL,media_category TEXT NOT NULL CHECK(media_category='image'),
 stored_width INTEGER NOT NULL CHECK(stored_width>0),stored_height INTEGER NOT NULL CHECK(stored_height>0),
 first_seq INTEGER NOT NULL REFERENCES publications(seq),CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'));
CREATE INDEX objects_sequence ON objects(first_seq,sha256);
CREATE TABLE acquisitions(acquisition_id TEXT PRIMARY KEY,context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),
 source_url TEXT NOT NULL,normalized_url TEXT NOT NULL,download_sha256 TEXT NOT NULL,
 download_md5 TEXT NOT NULL CHECK(length(download_md5)=32 AND download_md5 NOT GLOB '*[^0-9a-f]*'),
 download_bytes INTEGER NOT NULL CHECK(download_bytes>0),cdn_etag TEXT,acquired_at TEXT NOT NULL,
 evidence TEXT NOT NULL CHECK(evidence IN ('downloaded','http_validated','historical_reuse')),
 details_json TEXT NOT NULL CHECK(json_valid(details_json)),commit_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE INDEX acquisition_hash ON acquisitions(download_sha256,acquisition_id);
CREATE TABLE assets(asset_id TEXT PRIMARY KEY,media_id TEXT NOT NULL REFERENCES media_entries(media_id),
 acquisition_id TEXT NOT NULL REFERENCES acquisitions(acquisition_id),sha256 TEXT NOT NULL REFERENCES objects(sha256),
 representation TEXT NOT NULL CHECK(representation='original'),recipe_id TEXT NOT NULL CHECK(recipe_id='original'),
 acquired_at TEXT NOT NULL,commit_seq INTEGER NOT NULL REFERENCES publications(seq),UNIQUE(media_id,representation,recipe_id)) WITHOUT ROWID;
CREATE INDEX asset_object ON assets(sha256,asset_id);
CREATE INDEX asset_media ON assets(media_id,commit_seq,asset_id);
CREATE TABLE source_relations(relation_id TEXT PRIMARY KEY,pin_id TEXT NOT NULL REFERENCES pins(pin_id),
 entity_id TEXT NOT NULL REFERENCES source_entities(entity_id),role TEXT NOT NULL,capture_id TEXT NOT NULL REFERENCES captures(capture_id),
 commit_seq INTEGER NOT NULL REFERENCES publications(seq)) WITHOUT ROWID;
CREATE INDEX relations_pin ON source_relations(pin_id,role,entity_id,commit_seq);
CREATE INDEX relations_entity ON source_relations(entity_id,role,pin_id,commit_seq);
CREATE TABLE discovery_snapshots(snapshot_id TEXT PRIMARY KEY,capture_id TEXT NOT NULL REFERENCES captures(capture_id),
 entrypoint TEXT NOT NULL,scan_id TEXT NOT NULL,root_json TEXT NOT NULL CHECK(json_valid(root_json)),
 context_id TEXT NOT NULL REFERENCES visibility_contexts(context_id),observed_at TEXT NOT NULL,page_key TEXT NOT NULL,
 next_cursor_json TEXT CHECK(next_cursor_json IS NULL OR json_valid(next_cursor_json)),
 complete INTEGER NOT NULL CHECK(complete IN (0,1)),exhausted INTEGER NOT NULL CHECK(exhausted IN (0,1)),
 commit_seq INTEGER NOT NULL REFERENCES publications(seq),UNIQUE(scan_id,page_key)) WITHOUT ROWID;
CREATE TABLE discovery_members(snapshot_id TEXT NOT NULL REFERENCES discovery_snapshots(snapshot_id),
 ordinal INTEGER NOT NULL CHECK(ordinal>=0),pin_id TEXT NOT NULL REFERENCES pins(pin_id),
 commit_seq INTEGER NOT NULL REFERENCES publications(seq),PRIMARY KEY(snapshot_id,ordinal)) WITHOUT ROWID;
CREATE INDEX discovery_pin ON discovery_members(pin_id,snapshot_id);
CREATE TABLE changes(seq INTEGER NOT NULL REFERENCES publications(seq),sha256 TEXT NOT NULL REFERENCES objects(sha256),
 fields_json TEXT NOT NULL CHECK(json_valid(fields_json)),PRIMARY KEY(seq,sha256)) WITHOUT ROWID;
CREATE TABLE leases(id TEXT PRIMARY KEY,seq INTEGER NOT NULL,expires_ms INTEGER,owner TEXT NOT NULL,purpose TEXT NOT NULL) WITHOUT ROWID;
CREATE INDEX lease_sequence ON leases(seq,expires_ms);

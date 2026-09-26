
    PRAGMA application_id=1146310482;
    PRAGMA user_version=2;
    CREATE TABLE online_state(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
    CREATE TABLE publications(seq INTEGER PRIMARY KEY,batch_id TEXT NOT NULL UNIQUE,published_at TEXT NOT NULL,
      objects_count INTEGER NOT NULL,observations_count INTEGER NOT NULL,assets_count INTEGER NOT NULL);
    CREATE TABLE objects(object_row INTEGER PRIMARY KEY,sha256 TEXT NOT NULL UNIQUE,pack_path TEXT NOT NULL,
      offset INTEGER NOT NULL,length INTEGER NOT NULL,stored_ext TEXT,first_seq INTEGER NOT NULL);
    CREATE TABLE observations(row_id INTEGER PRIMARY KEY,"observation_id" TEXT,"post_id" INTEGER,"source_key" TEXT,"source_row" INTEGER,"archive_row" INTEGER,"source_kind" TEXT,"observed_at" TEXT,"time_quality" TEXT,"ingested_at" TEXT,"issues_json" TEXT,"publication_kind" TEXT,"publication_group" TEXT,"source_priority" INTEGER,"rating" TEXT,"tag_string" TEXT,"tag_string_general" TEXT,"tag_string_artist" TEXT,"tag_string_character" TEXT,"tag_string_copyright" TEXT,"tag_string_meta" TEXT,"source" TEXT,"md5" TEXT,"file_ext" TEXT,"file_url" TEXT,"large_file_url" TEXT,"preview_file_url" TEXT,"created_at" TEXT,"updated_at" TEXT,"score" INTEGER,"up_score" INTEGER,"down_score" INTEGER,"fav_count" INTEGER,"image_width" INTEGER,"image_height" INTEGER,"file_size" INTEGER,"tag_count" INTEGER,"uploader_id" INTEGER,"parent_id" INTEGER,"pixiv_id" INTEGER,"is_deleted" INTEGER,"is_banned" INTEGER,"is_pending" INTEGER,"is_flagged" INTEGER,batch_id TEXT,commit_seq INTEGER NOT NULL);
    CREATE UNIQUE INDEX observation_identity ON observations(observation_id);
    CREATE INDEX observation_post ON observations(post_id,row_id);
    CREATE TABLE assets(asset_row INTEGER PRIMARY KEY,"asset_id" TEXT,"observation_id" TEXT,"post_id" INTEGER,"sha256" TEXT,"source_md5" TEXT,"stored_ext" TEXT,"stored_bytes" INTEGER,"storage_profile" TEXT,"details_json" TEXT,batch_id TEXT,commit_seq INTEGER NOT NULL);
    CREATE UNIQUE INDEX asset_identity ON assets(asset_id);
    CREATE INDEX asset_object ON assets(sha256,asset_id);
    CREATE INDEX asset_post ON assets(post_id,source_md5,commit_seq,asset_id);
    CREATE TABLE raw_metadata(raw_row INTEGER PRIMARY KEY,observation_id TEXT NOT NULL UNIQUE,
      source_metadata_format TEXT,source_schema_id TEXT,raw_bytes INTEGER NOT NULL,
      raw_sha256 TEXT NOT NULL,raw_zlib BLOB NOT NULL);
    CREATE TABLE source_schemas(source_schema_id TEXT PRIMARY KEY,schema_ipc BLOB NOT NULL) WITHOUT ROWID;
    CREATE TABLE tags(tag_id INTEGER PRIMARY KEY,tag TEXT NOT NULL UNIQUE COLLATE BINARY);
    CREATE VIRTUAL TABLE tag_index USING fts5(tokens,content='',contentless_delete=1,detail='none',tokenize='ascii');
    CREATE VIRTUAL TABLE tag_vocabulary USING fts5vocab(tag_index,'row');
    CREATE TABLE post_versions(post_id INTEGER NOT NULL,valid_from INTEGER NOT NULL,valid_until INTEGER,
      row_id INTEGER NOT NULL,asset_id TEXT,PRIMARY KEY(post_id,valid_from)) WITHOUT ROWID;
    CREATE INDEX post_version_observation ON post_versions(row_id,valid_from,valid_until);
    CREATE INDEX post_version_asset ON post_versions(asset_id,valid_from,valid_until);
    CREATE TABLE object_versions(sha256 TEXT NOT NULL,valid_from INTEGER NOT NULL,valid_until INTEGER,
      post_id INTEGER,PRIMARY KEY(sha256,valid_from)) WITHOUT ROWID;
    CREATE INDEX object_post_order ON object_versions(post_id,sha256,valid_from,valid_until);
    CREATE TABLE changes(seq INTEGER NOT NULL,sha256 TEXT NOT NULL,fields TEXT NOT NULL,
      PRIMARY KEY(seq,sha256)) WITHOUT ROWID;
    CREATE TABLE leases(id TEXT PRIMARY KEY,seq INTEGER NOT NULL,expires_ms INTEGER,owner TEXT NOT NULL,
      purpose TEXT NOT NULL) WITHOUT ROWID;
    CREATE INDEX lease_sequence ON leases(seq,expires_ms);
    CREATE TABLE build_progress(name TEXT PRIMARY KEY,position INTEGER NOT NULL,rows INTEGER NOT NULL,
      digest TEXT NOT NULL,complete INTEGER NOT NULL DEFAULT 0) WITHOUT ROWID;
    CREATE TABLE pending_publication(seq INTEGER PRIMARY KEY,batch_id TEXT NOT NULL,fingerprint TEXT NOT NULL);
    CREATE INDEX objects_sequence ON objects(first_seq);
    CREATE INDEX observations_sequence ON observations(commit_seq,row_id);
    CREATE INDEX assets_sequence ON assets(commit_seq,asset_id);
    CREATE INDEX post_versions_expired ON post_versions(valid_until) WHERE valid_until IS NOT NULL;
    CREATE INDEX object_versions_expired ON object_versions(valid_until) WHERE valid_until IS NOT NULL;

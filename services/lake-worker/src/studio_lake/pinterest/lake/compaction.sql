CREATE TABLE IF NOT EXISTS archive_containers(
 container_id TEXT PRIMARY KEY,path TEXT NOT NULL UNIQUE,
 state TEXT NOT NULL CHECK(state IN ('building','sealed','active','cleaned')),
 first_seq INTEGER NOT NULL,last_seq INTEGER NOT NULL,
 bytes INTEGER NOT NULL CHECK(bytes>0),sha256 TEXT,
 created_at TEXT NOT NULL,verified_at TEXT,activated_at TEXT,cleaned_at TEXT,reference_json TEXT,
 CHECK(first_seq>0 AND last_seq>=first_seq));
CREATE INDEX IF NOT EXISTS archive_container_state ON archive_containers(state,first_seq);
CREATE TABLE IF NOT EXISTS archive_members(
 path TEXT PRIMARY KEY,container_id TEXT NOT NULL REFERENCES archive_containers(container_id),
 seq INTEGER NOT NULL REFERENCES commits(seq),
 offset INTEGER NOT NULL CHECK(offset>=512),length INTEGER NOT NULL CHECK(length>=0),
 sha256 TEXT NOT NULL CHECK(length(sha256)=64),
 copied INTEGER NOT NULL DEFAULT 0 CHECK(copied IN (0,1)),
 reclaimed INTEGER NOT NULL DEFAULT 0 CHECK(reclaimed IN (0,1)),
 UNIQUE(container_id,offset));
CREATE INDEX IF NOT EXISTS archive_member_order ON archive_members(container_id,offset);
CREATE INDEX IF NOT EXISTS archive_member_sequence ON archive_members(seq);

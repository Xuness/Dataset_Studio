CREATE TABLE query_families (
    id TEXT PRIMARY KEY,
    fingerprint TEXT,
    latest_revision INTEGER NOT NULL DEFAULT 0,
    latest_result_id TEXT,
    latest_count INTEGER NOT NULL DEFAULT 0,
    touched_at INTEGER NOT NULL,
    cached INTEGER NOT NULL DEFAULT 0,
    post_ready INTEGER NOT NULL DEFAULT 0,
    prune_pending INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX query_family_lookup ON query_families(fingerprint,touched_at DESC);
ALTER TABLE query_results ADD COLUMN family_id TEXT REFERENCES query_families(id);
ALTER TABLE query_results ADD COLUMN member_revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE query_results ADD COLUMN cache_mode TEXT NOT NULL DEFAULT 'full';
ALTER TABLE query_results ADD COLUMN cache_base TEXT;
ALTER TABLE query_results ADD COLUMN evaluated_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE query_results ADD COLUMN changed_members INTEGER NOT NULL DEFAULT 0;
ALTER TABLE query_results ADD COLUMN post_ready INTEGER NOT NULL DEFAULT 0;
ALTER TABLE query_results ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;
INSERT INTO query_families(id,latest_revision,latest_result_id,latest_count,touched_at)
SELECT id,1,id,COALESCE(count,0),CAST(created_at AS INTEGER) FROM query_results;
UPDATE query_results SET family_id=id;
CREATE INDEX query_result_family ON query_results(family_id,member_revision,status);
CREATE INDEX query_result_cache_mode ON query_results(cache_mode);
CREATE TABLE query_member_data (
    family_id TEXT NOT NULL REFERENCES query_families(id),
    source_id TEXT NOT NULL REFERENCES sources(id),
    asset_id TEXT NOT NULL,
    valid_from INTEGER NOT NULL,
    valid_until INTEGER,
    post_id INTEGER,
    PRIMARY KEY(family_id,source_id,asset_id,valid_from)
) WITHOUT ROWID;
INSERT INTO query_member_data(family_id,source_id,asset_id,valid_from)
SELECT result_id,source_id,asset_id,1 FROM result_members;
DROP TABLE result_members;
CREATE INDEX query_members_post ON query_member_data(family_id,post_id,source_id,asset_id,valid_from);
CREATE INDEX query_members_expired ON query_member_data(family_id,valid_until) WHERE valid_until IS NOT NULL;
CREATE VIEW result_members AS
SELECT r.id AS result_id,m.source_id,m.asset_id
FROM query_results r JOIN query_member_data m ON m.family_id=r.family_id
WHERE m.valid_from<=r.member_revision AND (m.valid_until IS NULL OR m.valid_until>r.member_revision);
CREATE TRIGGER query_result_default_family AFTER INSERT ON query_results
WHEN NEW.family_id IS NULL BEGIN
    INSERT INTO query_families(id,latest_revision,latest_result_id,latest_count,touched_at)
    VALUES(NEW.id,0,NULL,0,CAST(NEW.created_at AS INTEGER));
    UPDATE query_results SET family_id=NEW.id WHERE id=NEW.id;
END;
CREATE TRIGGER legacy_result_insert INSTEAD OF INSERT ON result_members BEGIN
    INSERT OR IGNORE INTO query_member_data(family_id,source_id,asset_id,valid_from)
    SELECT family_id,NEW.source_id,NEW.asset_id,member_revision FROM query_results WHERE id=NEW.result_id;
END;

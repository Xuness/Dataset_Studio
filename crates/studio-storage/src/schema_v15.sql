-- Existing members and bases remain immutable. Edits keep only changed keys;
-- validity intervals make an old scope readable without copying its members.
CREATE TABLE collection_membership_state(
    collection_id TEXT PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision >= 0)
) WITHOUT ROWID;
CREATE TABLE collection_versions(
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    count INTEGER NOT NULL CHECK(count >= 0),
    created_at TEXT NOT NULL,
    PRIMARY KEY(collection_id,revision)
) WITHOUT ROWID;
CREATE TABLE collection_member_changes(
    change_id INTEGER PRIMARY KEY,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id),
    asset_id TEXT NOT NULL,
    asset_blob BLOB GENERATED ALWAYS AS (unhex(asset_id)) VIRTUAL,
    valid_from INTEGER NOT NULL,
    valid_until INTEGER,
    present INTEGER NOT NULL CHECK(present IN (0,1)),
    ordinal INTEGER,
    input_json TEXT,
    scores_json TEXT,
    rating TEXT,
    post_id INTEGER,
    main_rank INTEGER,
    rescue_rank INTEGER,
    direct_rank INTEGER,
    fused_rank INTEGER,
    UNIQUE(collection_id,source_id,asset_id,valid_from),
    CHECK(valid_until IS NULL OR valid_until > valid_from)
);
CREATE UNIQUE INDEX collection_change_current ON collection_member_changes(collection_id,source_id,asset_id) WHERE valid_until IS NULL;
CREATE INDEX collection_change_version ON collection_member_changes(collection_id,valid_from,valid_until);
CREATE INDEX collection_change_ordinal ON collection_member_changes(collection_id,ordinal,valid_from,valid_until);
CREATE INDEX collection_change_source ON collection_member_changes(collection_id,source_id,present,valid_until,valid_from);
CREATE INDEX collection_change_main ON collection_member_changes(collection_id,present,coalesce(rating,'z'),coalesce(main_rank,9223372036854775807),ordinal);
CREATE INDEX collection_change_rescue ON collection_member_changes(collection_id,present,coalesce(rating,'z'),coalesce(rescue_rank,9223372036854775807),ordinal);
CREATE INDEX collection_change_direct ON collection_member_changes(collection_id,present,coalesce(rating,'z'),coalesce(direct_rank,9223372036854775807),ordinal);
CREATE INDEX collection_change_fused ON collection_member_changes(collection_id,present,coalesce(rating,'z'),coalesce(fused_rank,9223372036854775807),ordinal);
CREATE TABLE collection_edit_requests(
    request_id TEXT PRIMARY KEY,
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    request_json TEXT NOT NULL,
    response_json TEXT NOT NULL
) WITHOUT ROWID;
-- Ranking recipes can retain a version after the original collection is no
-- longer the active browser range. Reference traversal protects those owners.
CREATE TABLE collection_version_references(
    result_id TEXT NOT NULL REFERENCES query_results(id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections(id),
    revision INTEGER NOT NULL,
    PRIMARY KEY(result_id,collection_id,revision)
) WITHOUT ROWID;
CREATE INDEX collection_version_incoming ON collection_version_references(collection_id,result_id);
CREATE TRIGGER collection_version_release AFTER UPDATE OF status ON query_results WHEN NEW.status='released' BEGIN
    DELETE FROM collection_version_references WHERE result_id=NEW.id;
    DELETE FROM result_references WHERE owner_kind='ranking_recipe' AND owner_id=NEW.id;
END;
-- Before v15 a workset could not change; all existing results used version 0.
UPDATE query_results SET spec_json=json_set(spec_json,'$.input_scope.target.revision',0)
WHERE json_extract(spec_json,'$.input_scope.target.kind')='workset';

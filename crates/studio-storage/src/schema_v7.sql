ALTER TABLE query_families ADD COLUMN tier TEXT NOT NULL DEFAULT 'temporary';
ALTER TABLE query_families ADD COLUMN fixed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE query_families ADD COLUMN session_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE query_families ADD COLUMN stored_members INTEGER NOT NULL DEFAULT 0;
UPDATE query_families SET stored_members=(SELECT count(*) FROM query_member_data WHERE family_id=query_families.id);
UPDATE query_families SET tier='long_term'
WHERE EXISTS (
    SELECT 1 FROM query_results r WHERE r.id=query_families.latest_result_id
    AND json_extract(r.spec_json,'$.observation_rule')='current_post'
    AND COALESCE(json_extract(r.spec_json,'$.input_scope.target.kind'),'source')='source'
    AND json_array_length(r.spec_json,'$.conditions')>0
    AND NOT EXISTS(SELECT 1 FROM json_each(r.spec_json,'$.conditions') c
                   WHERE json_extract(c.value,'$.field')<>'rating')
);
CREATE TABLE query_cache_sessions (
    family_id TEXT NOT NULL REFERENCES query_families(id),
    session_id TEXT NOT NULL,
    PRIMARY KEY(family_id,session_id)
) WITHOUT ROWID;
ALTER TABLE query_results ADD COLUMN basis_ratings_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE query_results ADD COLUMN candidate_records INTEGER NOT NULL DEFAULT 0;

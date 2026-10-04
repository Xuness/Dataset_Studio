-- A ranking result can own a fixed, reproducible member recipe over immutable
-- ranking files. No asset identities or scores are copied into the project.
CREATE TABLE ranking_memberships(
    result_id TEXT PRIMARY KEY REFERENCES query_results(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    recipe_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX ranking_membership_recipe ON ranking_memberships(fingerprint);

-- A pending intent has never been authorized to dispatch. A materialized intent
-- must never recreate a missing ledger, which could erase paid attempt history.
CREATE TABLE evaluation_creation_intents (
 stage_id TEXT PRIMARY KEY REFERENCES evaluation_stage_refs(id),
 config TEXT NOT NULL CHECK(json_valid(config)),
 state TEXT NOT NULL CHECK(state IN ('pending','materialized','abandoned','cancelled')),
 created_at TEXT NOT NULL
);

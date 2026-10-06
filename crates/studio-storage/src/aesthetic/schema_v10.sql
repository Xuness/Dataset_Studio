-- Count only new, accepted comparative batches; historical evidence and explicit
-- candidate dispositions remain unchanged. A successful comparison resets this.
ALTER TABLE candidates ADD COLUMN unjudgeable_streak INTEGER NOT NULL DEFAULT 0
 CHECK(unjudgeable_streak>=0 AND unjudgeable_streak<=3);
PRAGMA user_version=10;

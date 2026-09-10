-- Display metadata never changes frozen computation material.
CREATE TABLE object_metadata (
    kind TEXT NOT NULL, id TEXT NOT NULL, name TEXT, notes TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0, created_at TEXT, updated_at TEXT,
    PRIMARY KEY(kind,id)
) WITHOUT ROWID;
INSERT INTO object_metadata(kind,id,created_at)
SELECT 'workset',c.id,NULL FROM collections c;
CREATE TABLE tool_presets (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, notes TEXT NOT NULL,
    revision INTEGER NOT NULL, run_json TEXT NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX tool_presets_operator ON tool_presets(json_extract(run_json,'$.operator_id'),name,id);
CREATE TABLE retired_workset_requests (
    request_id TEXT PRIMARY KEY, request_json TEXT NOT NULL,
    collection_id TEXT NOT NULL, retired_at TEXT NOT NULL
);

INSERT INTO meta VALUES ('selection_history_current','0');
CREATE TABLE selection_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, created_at TEXT NOT NULL,
    before_base TEXT, after_base TEXT, before_count INTEGER NOT NULL, after_count INTEGER NOT NULL,
    applied INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE selection_history_changes (
    step_id INTEGER NOT NULL REFERENCES selection_history(id) ON DELETE CASCADE,
    bucket TEXT NOT NULL, source_id TEXT NOT NULL, asset_id TEXT NOT NULL,
    before_present INTEGER NOT NULL, after_present INTEGER NOT NULL,
    PRIMARY KEY(step_id,bucket,source_id,asset_id)
) WITHOUT ROWID;
CREATE TABLE selection_history_refs (
    step_id INTEGER NOT NULL REFERENCES selection_history(id) ON DELETE CASCADE,
    phase INTEGER NOT NULL, kind TEXT NOT NULL, target_id TEXT NOT NULL,
    PRIMARY KEY(step_id,phase,kind,target_id)
) WITHOUT ROWID;
CREATE TRIGGER selection_history_add AFTER INSERT ON selection
WHEN (SELECT value FROM meta WHERE key='selection_history_current')!='0'
BEGIN
    INSERT INTO selection_history_changes VALUES
    ((SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_history_current'),'selection',NEW.source_id,NEW.asset_id,0,1)
    ON CONFLICT(step_id,bucket,source_id,asset_id) DO UPDATE SET after_present=1;
END;
CREATE TRIGGER selection_history_remove AFTER DELETE ON selection
WHEN (SELECT value FROM meta WHERE key='selection_history_current')!='0'
BEGIN
    INSERT INTO selection_history_changes VALUES
    ((SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_history_current'),'selection',OLD.source_id,OLD.asset_id,1,0)
    ON CONFLICT(step_id,bucket,source_id,asset_id) DO UPDATE SET after_present=0;
END;
CREATE TRIGGER selection_history_exclude AFTER INSERT ON selection_exclusions
WHEN (SELECT value FROM meta WHERE key='selection_history_current')!='0'
BEGIN
    INSERT INTO selection_history_changes VALUES
    ((SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_history_current'),'exclusion',NEW.source_id,NEW.asset_id,0,1)
    ON CONFLICT(step_id,bucket,source_id,asset_id) DO UPDATE SET after_present=1;
END;
CREATE TRIGGER selection_history_include AFTER DELETE ON selection_exclusions
WHEN (SELECT value FROM meta WHERE key='selection_history_current')!='0'
BEGIN
    INSERT INTO selection_history_changes VALUES
    ((SELECT CAST(value AS INTEGER) FROM meta WHERE key='selection_history_current'),'exclusion',OLD.source_id,OLD.asset_id,1,0)
    ON CONFLICT(step_id,bucket,source_id,asset_id) DO UPDATE SET after_present=0;
END;

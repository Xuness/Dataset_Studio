-- Per-attempt input evidence remains valid when a stage later changes image resolution.
ALTER TABLE attempts ADD COLUMN image_inputs TEXT;
PRAGMA user_version=12;

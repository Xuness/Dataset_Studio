-- User-arranged display order for offline jobs; existing jobs keep their id order.
ALTER TABLE analysis_jobs ADD COLUMN sort_key INTEGER NOT NULL DEFAULT 0;
UPDATE analysis_jobs SET sort_key=o.n FROM (SELECT id,ROW_NUMBER() OVER (ORDER BY id) AS n FROM analysis_jobs) AS o WHERE o.id=analysis_jobs.id;
PRAGMA user_version=13;

-- Only normalized receipts are scanned once. Raw bodies and images are untouched.
ALTER TABLE stages ADD COLUMN usage_summary TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(usage_summary));
CREATE TEMP TABLE usage_upgrade AS
WITH received AS (
 SELECT b.stage_id, json_extract(a.receipt,'$.usage.input_tokens') AS input,
  json_extract(a.receipt,'$.usage.cached_input_tokens') AS cached,
  json_extract(a.receipt,'$.usage.cache_write_tokens') AS written,
  json_extract(a.receipt,'$.usage.cost_usd') AS cost
 FROM attempts a JOIN batches b ON b.sequence=a.batch WHERE a.receipt IS NOT NULL
)
SELECT stage_id, json_object(
 'recorded_requests',count(*),
 'cache_observed_requests',sum(cached IS NOT NULL AND input IS NOT NULL),
 'cache_hit_requests',sum(CASE WHEN cached>0 AND input IS NOT NULL THEN 1 ELSE 0 END),
 'cached_input_tokens',sum(CASE WHEN cached IS NOT NULL AND input IS NOT NULL THEN cached ELSE 0 END),
 'cache_observed_input_tokens',sum(CASE WHEN cached IS NOT NULL AND input IS NOT NULL THEN input ELSE 0 END),
 'cache_write_observed_requests',sum(written IS NOT NULL),
 'cache_write_tokens',sum(COALESCE(written,0)),
 'cost_observed_requests',sum(CASE WHEN cost>=0 THEN 1 ELSE 0 END),
 'cost_usd',sum(CASE WHEN cost>=0 THEN cost ELSE 0 END)
) AS summary FROM received GROUP BY stage_id;
CREATE UNIQUE INDEX usage_upgrade_stage ON usage_upgrade(stage_id);
UPDATE stages SET usage_summary=COALESCE((SELECT summary FROM usage_upgrade WHERE stage_id=stages.id),'{}');
DROP TABLE usage_upgrade;
PRAGMA user_version=9;

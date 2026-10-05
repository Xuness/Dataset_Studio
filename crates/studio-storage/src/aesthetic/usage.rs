use super::*;
use studio_domain::llm::LlmUsage;

pub(super) fn replace(
    db: &Connection,
    stage: &str,
    old: Option<&LlmUsage>,
    new: &LlmUsage,
) -> Result<()> {
    let json: String = db
        .query_row(
            "SELECT usage_summary FROM stages WHERE id=?1",
            [stage],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let mut summary: AestheticUsageSummary = decode(json)?;
    if let Some(old) = old {
        apply(&mut summary, old, false)?;
    }
    apply(&mut summary, new, true)?;
    db.execute(
        "UPDATE stages SET usage_summary=?2 WHERE id=?1",
        params![stage, encode(&summary)?],
    )
    .map_err(db_error)?;
    Ok(())
}

fn apply(s: &mut AestheticUsageSummary, usage: &LlmUsage, add: bool) -> Result<()> {
    let update = |target: &mut u64, value: u64| -> Result<()> {
        *target = if add {
            target.checked_add(value)
        } else {
            target.checked_sub(value)
        }
        .ok_or_else(|| Error::new("EVALUATION_CORRUPT", "缓存用量统计计数越界"))?;
        Ok(())
    };
    update(&mut s.recorded_requests, 1)?;
    if let (Some(cached), Some(input)) = (usage.cached_input_tokens, usage.input_tokens) {
        update(&mut s.cache_observed_requests, 1)?;
        update(&mut s.cache_hit_requests, u64::from(cached > 0))?;
        update(&mut s.cached_input_tokens, cached)?;
        update(&mut s.cache_observed_input_tokens, input)?;
    }
    if let Some(written) = usage.cache_write_tokens {
        update(&mut s.cache_write_observed_requests, 1)?;
        update(&mut s.cache_write_tokens, written)?;
    }
    if let Some(cost) = usage.cost_usd.filter(|v| v.is_finite() && *v >= 0.0) {
        update(&mut s.cost_observed_requests, 1)?;
        s.cost_usd = (s.cost_usd + if add { cost } else { -cost }).max(0.0);
    }
    Ok(())
}

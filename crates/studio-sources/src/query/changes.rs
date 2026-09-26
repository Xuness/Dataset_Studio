use super::*;

pub use studio_domain::ChangeAnchor;

/// The supported producer never overwrites observations/assets in a generation.
/// It records their commit_seq and rebuilds current_posts for affected post IDs.
/// A new generation, missing columns/history, or changed anchor forces a rebuild.
pub(crate) fn anchor(db: &Session, catalog: &Catalog) -> Result<Option<ChangeAnchor>> {
    let supported = db.query("SELECT count(*) FROM information_schema.columns WHERE (table_name='observations' AND column_name='commit_seq') OR (table_name='assets' AND column_name='commit_seq') OR (table_name='applied' AND column_name='batch_id') OR (table_name='objects' AND column_name='pack_path')")?;
    if supported[0][0].as_deref() != Some("4") {
        return Ok(None);
    }
    let rows = db.query(&format!(
        "SELECT batch_id FROM applied WHERE seq={}",
        catalog.sequence
    ))?;
    Ok(rows
        .first()
        .and_then(|r| r[0].clone())
        .map(|batch_id| ChangeAnchor {
            generation: catalog.generation.clone(),
            sequence: catalog.sequence,
            batch_id,
        }))
}

pub(crate) fn change_sql(
    db: &Session,
    catalog: &Catalog,
    previous: &ChangeAnchor,
) -> Result<Option<String>> {
    if previous.generation != catalog.generation || previous.sequence > catalog.sequence {
        return Ok(None);
    }
    if anchor(db, catalog)?.is_none() {
        return Ok(None);
    }
    let rows = db.query(&format!(
        "SELECT batch_id FROM applied WHERE seq={}",
        previous.sequence
    ))?;
    if rows.first().and_then(|r| r[0].as_deref()) != Some(previous.batch_id.as_str()) {
        return Ok(None);
    }
    let count = db.query(&format!(
        "SELECT count(*) FROM applied WHERE seq>{} AND seq<={}",
        previous.sequence, catalog.sequence
    ))?;
    if count[0][0].as_deref().and_then(|s| s.parse::<u64>().ok())
        != Some(catalog.sequence - previous.sequence)
    {
        return Ok(None);
    }
    let from = previous.sequence;
    let to = catalog.sequence;
    Ok(Some(format!(
        "WITH changed_observations AS (SELECT observation_id,post_id FROM observations WHERE commit_seq>{from} AND commit_seq<={to}), affected_posts AS (SELECT post_id FROM changed_observations WHERE post_id IS NOT NULL UNION SELECT post_id FROM assets WHERE commit_seq>{from} AND commit_seq<={to} AND post_id IS NOT NULL) SELECT sha256 FROM assets WHERE sha256 IS NOT NULL AND ((commit_seq>{from} AND commit_seq<={to}) OR post_id IN (SELECT post_id FROM affected_posts) OR observation_id IN (SELECT observation_id FROM changed_observations)) UNION SELECT sha256 FROM objects WHERE pack_path IN (SELECT 'segments/'||batch_id||'/images.tar' FROM applied WHERE seq>{from} AND seq<={to})"
    )))
}

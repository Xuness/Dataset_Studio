use super::*;
use studio_domain::visual::{VisualMediaPage, VisualMediaRef};

pub(crate) fn namespace(
    source: &Source,
    expected: &QuerySourceVersion,
    cancelled: studio_application::ReadCancellation,
) -> Result<String> {
    let view = Snapshot::open(
        source,
        Some(&expected.catalog_revision),
        cancelled,
        Some(Instant::now() + Duration::from_secs(30)),
    )?;
    if view.version(source) != *expected {
        return Err(Error::new("SOURCE_CHANGED", "视觉来源版本不一致"));
    }
    if view.pointer.schema_version != 2
        || !matches!(source.kind.as_str(), "danbooru" | "yandere" | "gelbooru")
    {
        return Err(Error::new(
            "SOURCE_FORMAT_UNSUPPORTED",
            "视觉增量发现仅支持三个 Booru 在线数据湖",
        ));
    }
    Ok(format!("online-v2:{}", view.pointer.generation))
}

pub(crate) fn page(
    source: &Source,
    expected: &QuerySourceVersion,
    after_row: u64,
    limit: u32,
    cancelled: studio_application::ReadCancellation,
) -> Result<VisualMediaPage> {
    query_page(source, expected, after_row, limit, None, cancelled)
}

pub(crate) fn selected(
    source: &Source,
    expected: &QuerySourceVersion,
    assets: &[String],
    cancelled: studio_application::ReadCancellation,
) -> Result<VisualMediaPage> {
    if assets.is_empty()
        || assets.len() > 4096
        || assets
            .iter()
            .any(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::invalid("指定图片需要 1-4096 个 SHA 身份"));
    }
    let page = query_page(source, expected, 0, 4096, Some(assets), cancelled)?;
    if page.items.len() != assets.len() {
        return Err(Error::new(
            "NOT_FOUND",
            "指定图片在固定来源版本中缺失或重复",
        ));
    }
    Ok(page)
}

fn query_page(
    source: &Source,
    expected: &QuerySourceVersion,
    after_row: u64,
    limit: u32,
    assets: Option<&[String]>,
    cancelled: studio_application::ReadCancellation,
) -> Result<VisualMediaPage> {
    if !matches!(source.kind.as_str(), "danbooru" | "yandere" | "gelbooru") || !available(source) {
        return Err(Error::new(
            "SOURCE_FORMAT_UNSUPPORTED",
            "视觉后端当前支持三个 Booru 在线数据湖",
        ));
    }
    if limit == 0 || limit > 4096 || after_row > i64::MAX as u64 {
        return Err(Error::invalid("视觉目录分页超出范围"));
    }
    let view = Snapshot::open(
        source,
        Some(&expected.catalog_revision),
        cancelled,
        Some(Instant::now() + Duration::from_secs(30)),
    )?;
    if view.version(source) != *expected {
        return Err(Error::new("SOURCE_CHANGED", "视觉来源版本不一致"));
    }
    let sql = if assets.is_some() {
        "SELECT object_row,sha256,pack_path,offset,length,coalesce(stored_ext,'') FROM visible_objects WHERE sha256 IN (SELECT value FROM json_each(?3)) AND object_row>?1 ORDER BY object_row LIMIT ?2"
    } else {
        "SELECT object_row,sha256,pack_path,offset,length,coalesce(stored_ext,'') FROM visible_objects WHERE object_row>?1 AND ?3 IS NULL ORDER BY object_row LIMIT ?2"
    };
    let mut query = view.db.prepare(sql).map_err(sql_error)?;
    let selected_json = assets
        .map(serde_json::to_string)
        .transpose()
        .map_err(error)?;
    let mut rows = query
        .query(params![
            after_row as i64,
            i64::from(limit) + 1,
            selected_json
        ])
        .map_err(sql_error)?;
    let mut items = Vec::with_capacity(limit as usize + 1);
    while let Some(row) = rows.next().map_err(sql_error)? {
        let item = VisualMediaRef {
            row_id: u64::try_from(row.get::<_, i64>(0).map_err(sql_error)?).map_err(error)?,
            sha256: row.get(1).map_err(sql_error)?,
            pack: row.get(2).map_err(sql_error)?,
            offset: u64::try_from(row.get::<_, i64>(3).map_err(sql_error)?).map_err(error)?,
            length: u64::try_from(row.get::<_, i64>(4).map_err(sql_error)?).map_err(error)?,
            extension: row.get(5).map_err(sql_error)?,
        };
        if item.sha256.len() != 64
            || !item
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || Path::new(&item.pack)
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
            || item.offset.checked_add(item.length).is_none()
        {
            return Err(Error::new("SOURCE_CORRUPT", "视觉对象的哈希或物理范围无效"));
        }
        items.push(item);
    }
    let complete = items.len() <= limit as usize;
    items.truncate(limit as usize);
    Ok(VisualMediaPage { items, complete })
}

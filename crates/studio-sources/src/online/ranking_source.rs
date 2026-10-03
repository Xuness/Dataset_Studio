//! Full-source projection scans wide SQLite relations in row order. Only narrow
//! identity links and dimensions live in memory; output remains bounded pages.
use super::*;
use std::{collections::VecDeque, thread};
use studio_application::ReadCancellation;

const PAGE: i64 = 32768;
const NONE: u32 = u32::MAX;

struct Object {
    hash: [u8; 32],
    bytes: u64,
    extension: u16,
    candidates: u32,
    emitted: bool,
}
struct Asset {
    hash: [u8; 32],
    origin: [u8; 32],
    object: u32,
    width: u32,
    height: u32,
    dimension_basis: u8,
}
struct Raw {
    asset: u32,
    size: u64,
    compressed: Vec<u8>,
    digest: String,
}
type Size = Option<(u32, u32)>;
type Decoded = (u32, std::result::Result<Size, ()>);

fn hash(value: &str) -> Result<[u8; 32]> {
    let mut result = [0; 32];
    hex::decode_to_slice(value, &mut result).map_err(error)?;
    Ok(result)
}
fn canonical_hash(value: &str) -> Option<[u8; 32]> {
    (value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| hash(value).ok())
    .flatten()
}
fn open(
    source: &Source,
    expected: &QuerySourceVersion,
    flag: &ReadCancellation,
) -> Result<Snapshot> {
    let view = Snapshot::open_bulk(
        source,
        Some(&expected.catalog_revision),
        flag.clone(),
        Some(Instant::now() + Duration::from_secs(60)),
    )?;
    if view.version(source) != *expected {
        return Err(Error::new("SOURCE_CHANGED", "全湖投影版本不一致"));
    }
    Ok(view)
}
fn log(start: Instant, phase: &str, count: usize) {
    tracing::info!(
        phase,
        count,
        elapsed_ms = start.elapsed().as_millis(),
        "ranking source scan"
    );
}

pub(crate) struct Population {
    objects: Vec<Object>,
    extensions: Vec<String>,
    assets: Vec<Asset>,
    observation_assets: Vec<u32>,
    max_observation: i64,
}

impl Population {
    pub(crate) fn load(
        source: &Source,
        expected: &QuerySourceVersion,
        memory: u64,
        dimensions: bool,
        flag: &ReadCancellation,
    ) -> Result<Option<Self>> {
        let start = Instant::now();
        let view = open(source, expected, flag)?;
        let total = view.count;
        let positive: bool=view.db.query_row("SELECT coalesce((SELECT min(object_row) FROM objects),1)>0 AND coalesce((SELECT min(asset_row) FROM assets),1)>0 AND coalesce((SELECT min(raw_row) FROM raw_metadata),1)>0",[],|r|r.get(0)).map_err(sql_error)?;
        if !positive || usize::BITS < 64 {
            return Ok(None);
        }
        let (asset_count, max_observation): (i64,i64) = view.db.query_row(
            "SELECT assets_count,(SELECT coalesce(max(row_id),0) FROM observations) FROM publications WHERE seq=?1",
            [view.sequence as i64], |r| Ok((r.get(0)?,r.get(1)?)),
        ).map_err(sql_error)?;
        // Conservative allowance includes hash-table spare capacity, temporary
        // keys, decoder pages and writer buffers. Sparse/outsize sources use
        // the existing bounded projection instead of an unbounded allocation.
        let needed = total
            .saturating_mul(128)
            .saturating_add((asset_count.max(0) as u64).saturating_mul(240))
            .saturating_add((max_observation.max(0) as u64).saturating_mul(4))
            .saturating_add(1 << 30);
        if total >= u64::from(NONE)
            || asset_count < 0
            || asset_count as u64 >= u64::from(NONE)
            || max_observation < 0
            || max_observation as u64 >= u64::from(NONE)
            || needed > memory
        {
            return Ok(None);
        }
        drop(view);
        let mut objects = Vec::with_capacity(total as usize);
        let mut extensions = vec![String::new()];
        let mut after = 0i64;
        let mut view = open(source, expected, flag)?;
        loop {
            view.next_bulk_page()?;
            let mut stmt = view.db.prepare("SELECT object_row,sha256,length,stored_ext FROM visible_objects WHERE object_row>?1 ORDER BY object_row LIMIT ?2").map_err(sql_error)?;
            let mut rows = stmt.query(params![after, PAGE]).map_err(sql_error)?;
            let old = after;
            while let Some(row) = rows.next().map_err(sql_error)? {
                after = row.get(0).map_err(sql_error)?;
                let ext = row
                    .get::<_, Option<String>>(3)
                    .map_err(sql_error)?
                    .unwrap_or_default();
                let extension = if let Some(i) = extensions.iter().position(|s| s == &ext) {
                    i
                } else {
                    if extensions.len() >= u16::MAX as usize {
                        return Err(error("存储扩展名种类超出预算"));
                    }
                    extensions.push(ext);
                    extensions.len() - 1
                } as u16;
                let Some(key) =
                    canonical_hash(row.get_ref(1).map_err(sql_error)?.as_str().map_err(error)?)
                else {
                    return Ok(None);
                };
                objects.push(Object {
                    hash: key,
                    bytes: u64::try_from(row.get::<_, i64>(2).map_err(sql_error)?)
                        .map_err(error)?,
                    extension,
                    candidates: 0,
                    emitted: false,
                });
                if objects.len() as u64 > total {
                    return Err(error("全湖成员超过固定发布数量"));
                }
            }
            if after == old {
                break;
            }
        }
        if objects.len() as u64 != total {
            return Err(error("全湖成员与固定发布数量不一致"));
        }
        objects.sort_unstable_by_key(|o| o.hash);
        let object_index: HashMap<_, _> = objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.hash, i as u32))
            .collect();
        if object_index.len() != objects.len() {
            return Err(error("全湖图片身份重复"));
        }
        log(start, "objects", objects.len());
        let mut assets = Vec::with_capacity(asset_count as usize);
        let mut asset_index = HashMap::with_capacity(asset_count as usize);
        after = 0;
        loop {
            view.next_bulk_page()?;
            let mut stmt = view.db.prepare("SELECT asset_row,asset_id,observation_id,sha256,details_json FROM visible_assets WHERE asset_row>?1 ORDER BY asset_row LIMIT ?2").map_err(sql_error)?;
            let mut rows = stmt.query(params![after, PAGE]).map_err(sql_error)?;
            let old = after;
            while let Some(row) = rows.next().map_err(sql_error)? {
                after = row.get(0).map_err(sql_error)?;
                let Some(key) = row.get::<_, Option<String>>(3).map_err(sql_error)? else {
                    continue;
                };
                let Some(key) = canonical_hash(&key) else {
                    continue;
                };
                let Some(&object) = object_index.get(&key) else {
                    continue;
                };
                let size = if dimensions {
                    match row.get_ref(4).map_err(sql_error)? {
                        rusqlite::types::ValueRef::Null => Ok(None),
                        rusqlite::types::ValueRef::Text(value) if value.len() <= 64 << 10 => {
                            std::str::from_utf8(value)
                                .map_err(|_| ())
                                .and_then(|s| super::ranking_simple::dimensions(s, ""))
                        }
                        _ => Err(()),
                    }
                } else {
                    Ok(None)
                };
                let (width, height, basis) = match size {
                    Ok(Some((w, h))) => (w, h, 1),
                    Ok(None) => (0, 0, 0),
                    Err(()) => (0, 0, 3),
                };
                let Some(key) = row.get::<_, Option<String>>(1).map_err(sql_error)? else {
                    continue;
                };
                let Some(key) = canonical_hash(&key) else {
                    return Ok(None);
                };
                let origin: Option<String> = row.get(2).map_err(sql_error)?;
                let origin_hash = match origin.as_deref() {
                    Some(value) => match canonical_hash(value) {
                        Some(key) => key,
                        None => return Ok(None),
                    },
                    None => [0; 32],
                };
                if origin.is_none() || basis == 3 {
                    objects[object as usize].candidates = 2;
                }
                if assets.len() >= asset_count as usize {
                    return Err(error("来源资产超过固定发布数量"));
                }
                if asset_index.insert(key, assets.len() as u32).is_some() {
                    return Err(error("来源资产身份重复"));
                }
                assets.push(Asset {
                    hash: key,
                    origin: origin_hash,
                    object,
                    width,
                    height,
                    dimension_basis: basis,
                });
            }
            if after == old {
                break;
            }
        }
        drop(object_index);
        log(start, "assets", assets.len());
        let mut observation_assets = vec![NONE; max_observation as usize + 1];
        let mut after_post = (i64::MIN, -1i64);
        loop {
            view.next_bulk_page()?;
            let mut stmt = view.db.prepare("SELECT post_id,row_id,asset_id,valid_from FROM post_versions WHERE (post_id,valid_from)>(?1,?2) AND valid_from<=?3 AND (valid_until IS NULL OR valid_until>?3) ORDER BY post_id,valid_from LIMIT ?4").map_err(sql_error)?;
            let mut rows = stmt
                .query(params![
                    after_post.0,
                    after_post.1,
                    view.sequence as i64,
                    PAGE
                ])
                .map_err(sql_error)?;
            let old = after_post;
            while let Some(row) = rows.next().map_err(sql_error)? {
                after_post = (
                    row.get::<_, i64>(0).map_err(sql_error)?,
                    row.get::<_, i64>(3).map_err(sql_error)?,
                );
                let key: Option<String> = row.get(2).map_err(sql_error)?;
                let Some(key) = key else {
                    continue;
                };
                let Some(key) = canonical_hash(&key) else {
                    continue;
                };
                let Some(&asset) = asset_index.get(&key) else {
                    continue;
                };
                let obs: i64 = row.get(1).map_err(sql_error)?;
                let object = assets[asset as usize].object as usize;
                objects[object].candidates = objects[object].candidates.saturating_add(1);
                if obs < 0 || obs > max_observation {
                    objects[object].candidates = 2;
                    continue;
                }
                let prior = observation_assets[obs as usize];
                if prior != NONE {
                    objects[object].candidates = 2;
                    objects[assets[prior as usize].object as usize].candidates = 2;
                } else {
                    observation_assets[obs as usize] = asset;
                }
            }
            if after_post == old {
                break;
            }
        }
        drop(asset_index);
        drop(view);
        log(start, "current_posts", observation_assets.len());
        let mut population = Self {
            objects,
            extensions,
            assets,
            observation_assets,
            max_observation,
        };
        if dimensions {
            population.read_dimensions(source, expected, flag)?;
        }
        Ok(Some(population))
    }

    fn read_dimensions(
        &mut self,
        source: &Source,
        expected: &QuerySourceVersion,
        flag: &ReadCancellation,
    ) -> Result<()> {
        let start = Instant::now();
        let mut origins = HashMap::new();
        let mut next = vec![NONE; self.assets.len()];
        for asset in self
            .observation_assets
            .iter()
            .copied()
            .filter(|v| *v != NONE)
        {
            let a = &self.assets[asset as usize];
            if a.dimension_basis == 0
                && self.objects[a.object as usize].candidates == 1
                && let Some(prior) = origins.insert(a.origin, asset)
            {
                next[asset as usize] = prior;
            }
        }
        if origins.is_empty() {
            log(start, "raw_dimensions", 0);
            return Ok(());
        }
        let apply = |this: &mut Self, decoded: Vec<Decoded>| {
            for (mut at, size) in decoded {
                while at != NONE {
                    let a = &mut this.assets[at as usize];
                    match size {
                        Ok(Some((w, h))) => {
                            a.width = w;
                            a.height = h;
                            a.dimension_basis = 2;
                        }
                        Ok(None) => (),
                        Err(()) => this.objects[a.object as usize].candidates = 2,
                    }
                    at = next[at as usize];
                }
            }
        };
        let workers = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, 8);
        thread::scope(|scope| -> Result<()> {
            let mut pending = VecDeque::new();
            let mut after = 0i64;
            let mut view = open(source, expected, flag)?;
            loop {
                view.next_bulk_page()?;
                let mut stmt = view.db.prepare("SELECT raw_row,observation_id,raw_bytes,raw_sha256,raw_zlib FROM raw_metadata WHERE raw_row>?1 ORDER BY raw_row LIMIT ?2").map_err(sql_error)?;
                let mut rows = stmt.query(params![after, PAGE]).map_err(sql_error)?;
                let old = after;
                let mut batch = Vec::new();
                let mut bytes = 0usize;
                while let Some(row) = rows.next().map_err(sql_error)? {
                    after = row.get(0).map_err(sql_error)?;
                    let Some(key) =
                        canonical_hash(row.get_ref(1).map_err(sql_error)?.as_str().map_err(error)?)
                    else {
                        continue;
                    };
                    let Some(&asset) = origins.get(&key) else {
                        continue;
                    };
                    let size: i64 = row.get(2).map_err(sql_error)?;
                    if size < 0 {
                        continue;
                    }
                    if size > 16 << 20 {
                        return Err(Error::new(
                            "READ_BUDGET_EXCEEDED",
                            "排名原始尺寸记录超过 16 MiB 预算",
                        ));
                    }
                    let compressed: Vec<u8> = row.get(4).map_err(sql_error)?;
                    bytes += compressed.len();
                    batch.push(Raw {
                        asset,
                        size: size as u64,
                        compressed,
                        digest: row.get(3).map_err(sql_error)?,
                    });
                    if bytes >= 32 << 20 {
                        break;
                    }
                }
                drop(rows);
                drop(stmt);
                if after == old {
                    break;
                }
                let flag = flag.clone();
                pending.push_back(scope.spawn(move || -> Result<Vec<Decoded>> {
                    let mut decoded = Vec::with_capacity(batch.len());
                    for raw in batch {
                        studio_application::read_cancelled(&flag)?;
                        let body =
                            super::raw::decode(&raw.compressed, raw.size, &raw.digest, 16 << 20)?;
                        let size: super::dimensions::Dimensions =
                            serde_json::from_str(&body).map_err(error)?;
                        let size = serde_json::to_string(&size).map_err(error)?;
                        decoded.push((raw.asset, super::ranking_simple::dimensions(&size, "raw_")));
                    }
                    Ok(decoded)
                }));
                if pending.len() >= workers {
                    let decoded = pending
                        .pop_front()
                        .expect("pending raw projection")
                        .join()
                        .map_err(|_| error("尺寸解码线程异常"))??;
                    apply(self, decoded);
                }
            }
            while let Some(worker) = pending.pop_front() {
                apply(self, worker.join().map_err(|_| error("尺寸解码线程异常"))??);
            }
            Ok(())
        })?;
        log(start, "raw_dimensions", origins.len());
        Ok(())
    }

    pub(crate) fn project(
        &mut self,
        source: &Source,
        expected: &QuerySourceVersion,
        parameters: &RankingParameters,
        flag: &ReadCancellation,
        sink: &mut dyn FnMut(&[RankingInput]) -> Result<()>,
    ) -> Result<Vec<(u64, String, u32)>> {
        let start = Instant::now();
        let workers = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .clamp(1, 8);
        let width = (self.max_observation + workers as i64 - 1) / workers as i64;
        let mut seen = vec![false; self.objects.len()];
        let population = &*self;
        thread::scope(|scope| -> Result<()> {
            let (send, receive) =
                std::sync::mpsc::sync_channel::<Result<Vec<RankingInput>>>(workers * 2);
            for index in 0..workers {
                let send = send.clone();
                let lower = index as i64 * width;
                let upper = ((index + 1) as i64 * width).min(population.max_observation);
                scope.spawn(move || {
                    let outcome = population
                        .project_range(source, expected, parameters, flag, lower, upper, &send);
                    if let Err(e) = outcome {
                        let _ = send.send(Err(e));
                    }
                });
            }
            drop(send);
            let outcome = (|| -> Result<()> {
                for page in &receive {
                    let page = page?;
                    for row in &page {
                        let prior = seen
                            .get_mut(row.ordinal as usize)
                            .ok_or_else(|| error("全湖排名序号越界"))?;
                        if *prior {
                            return Err(error("全湖排名成员重复"));
                        }
                        *prior = true;
                    }
                    sink(&page)?;
                }
                Ok(())
            })();
            if outcome.is_err() {
                flag.store(true, Ordering::Release);
            }
            // Release blocked senders before joining scoped workers on error.
            drop(receive);
            outcome
        })?;
        let mut page = Vec::with_capacity(512);
        let mut fallback = Vec::new();
        for (ordinal, object) in self.objects.iter_mut().enumerate() {
            if seen[ordinal] {
                object.emitted = true;
                continue;
            }
            if object.candidates != 0 {
                fallback.push((ordinal as u64, hex::encode(object.hash), 0));
                continue;
            }
            page.push(RankingInput {
                ordinal: ordinal as u64,
                source_id: source.id.clone(),
                asset_id: hex::encode(object.hash),
                time_quality: "unknown".into(),
                dimension_basis: "not_recorded".into(),
                stored_extension: self.extensions[object.extension as usize].clone(),
                stored_bytes: object.bytes,
                ..Default::default()
            });
            object.emitted = true;
            if page.len() == 512 {
                sink(&page)?;
                page.clear();
            }
        }
        if !page.is_empty() {
            sink(&page)?;
        }
        log(start, "observations", self.objects.len());
        Ok(fallback)
    }

    #[allow(clippy::too_many_arguments)]
    fn project_range(
        &self,
        source: &Source,
        expected: &QuerySourceVersion,
        parameters: &RankingParameters,
        flag: &ReadCancellation,
        mut lower: i64,
        upper: i64,
        send: &std::sync::mpsc::SyncSender<Result<Vec<RankingInput>>>,
    ) -> Result<()> {
        if lower >= upper {
            return Ok(());
        }
        let mut view = open(source, expected, flag)?;
        let mut page = Vec::with_capacity(512);
        while lower < upper {
            view.next_bulk_page()?;
            let end = (lower + PAGE).min(upper);
            let mut stmt=view.db.prepare("SELECT p.row_id,NULL,0,NULL,NULL,NULL,p.observation_id,p.post_id,p.rating,p.created_at,p.observed_at,p.updated_at,p.time_quality,p.source_priority,p.fav_count,p.up_score,p.down_score,p.score,p.tag_string_artist,p.tag_string,p.parent_id,p.is_banned,p.is_deleted,p.is_pending,p.is_flagged,p.issues_json,p.row_id FROM visible_observations p WHERE p.row_id>?1 AND p.row_id<=?2 ORDER BY p.row_id").map_err(sql_error)?;
            let mut rows = stmt.query(params![lower, end]).map_err(sql_error)?;
            while let Some(row) = rows.next().map_err(sql_error)? {
                let at = self.observation_assets[row.get::<_, i64>(0).map_err(sql_error)? as usize];
                if at == NONE {
                    continue;
                }
                let asset = &self.assets[at as usize];
                let object = &self.objects[asset.object as usize];
                if object.candidates != 1 {
                    continue;
                }
                if !super::ranking_simple::row_fits(row)? {
                    continue;
                }
                let mut c = super::ranking_simple::read_candidate(
                    row,
                    source,
                    u64::from(asset.object),
                    hex::encode(object.hash),
                    parameters,
                )?;
                let (Ok(created), Ok(observed), Ok(updated)) = (
                    super::ranking_simple::timestamp(c.dates[0].as_deref()),
                    super::ranking_simple::timestamp(c.dates[1].as_deref()),
                    super::ranking_simple::timestamp(c.dates[2].as_deref()),
                ) else {
                    continue;
                };
                c.input.record_id = Some(hex::encode(asset.hash));
                (
                    c.input.created_at_us,
                    c.input.observed_at_us,
                    c.input.updated_at_us,
                ) = (created, observed, updated);
                c.input.stored_extension = self.extensions[object.extension as usize].clone();
                c.input.stored_bytes = object.bytes;
                c.input.stored_width = (asset.width > 0).then_some(asset.width);
                c.input.stored_height = (asset.height > 0).then_some(asset.height);
                c.input.dimension_basis = if parameters.minimum_stored_side.is_none() {
                    "not_requested"
                } else {
                    match asset.dimension_basis {
                        1 => "asset_storage_details",
                        2 => "asset_origin_raw_metadata",
                        _ => "not_recorded",
                    }
                }
                .into();
                page.push(c.input);
                if page.len() == 512 {
                    send.send(Ok(std::mem::replace(&mut page, Vec::with_capacity(512))))
                        .map_err(|_| Error::new("CANCELLED", "排名输入接收已停止"))?;
                    studio_application::read_cancelled(flag)?;
                }
            }
            lower = end;
        }
        if !page.is_empty() {
            send.send(Ok(page))
                .map_err(|_| Error::new("CANCELLED", "排名输入接收已停止"))?;
        }
        Ok(())
    }
}

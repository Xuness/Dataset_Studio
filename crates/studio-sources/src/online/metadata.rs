use super::*;
use crate::duckdb::Session;
use rusqlite::functions::FunctionFlags;
use sha2::{Digest, Sha256};

pub(crate) enum MetadataConnection {
    Native(Session),
    Online(Box<Snapshot>),
}
impl MetadataConnection {
    pub fn online(snapshot: Snapshot) -> Result<Self> {
        snapshot
            .db
            .create_scalar_function(
                "year",
                1,
                FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_UTF8,
                |ctx| {
                    let value: Option<String> = ctx.get(0)?;
                    Ok(value
                        .as_deref()
                        .and_then(|v| v.get(..4))
                        .and_then(|v| v.parse::<i64>().ok()))
                },
            )
            .map_err(sql_error)?;
        snapshot
            .db
            .execute_batch(
                "CREATE TEMP VIEW assets AS SELECT * FROM visible_assets;
            CREATE TEMP VIEW observations AS SELECT * FROM visible_observations;",
            )
            .map_err(sql_error)?;
        snapshot
            .db
            .execute_batch(&format!(
                "CREATE TEMP VIEW applied AS SELECT seq,batch_id FROM publications WHERE seq<={}",
                snapshot.sequence
            ))
            .map_err(sql_error)?;
        Ok(Self::Online(Box::new(snapshot)))
    }
    pub fn query(&self, sql: &str) -> Result<Vec<Vec<Option<String>>>> {
        match self {
            Self::Native(db) => db.query(sql),
            Self::Online(db) => db.strings(sql, 2048),
        }
    }
    pub fn begin(&self) -> Result<()> {
        if let Self::Native(db) = self {
            db.query("BEGIN TRANSACTION")?;
        }
        Ok(())
    }
    pub fn query_bounded(&self, sql: &str, limit: u64) -> Result<Vec<Vec<Option<String>>>> {
        match self {
            Self::Native(db) => db.query_bounded(sql, limit),
            Self::Online(db) => db.strings(sql, limit as usize),
        }
    }
    pub fn raw(&self, id: &str) -> Result<Vec<Vec<Option<String>>>> {
        match self {
            Self::Native(db)=>db.query(&format!("SELECT source_metadata_format,source_schema_id,CAST(octet_length(encode(source_metadata_json)) AS VARCHAR),CASE WHEN octet_length(encode(source_metadata_json))<=131072 THEN source_metadata_json ELSE NULL END FROM raw_metadata WHERE observation_id='{}' LIMIT 1",id.replace('\'',"''"))),
            Self::Online(snapshot)=>{
                let mut statement=snapshot.db.prepare("SELECT source_metadata_format,source_schema_id,raw_bytes,raw_sha256,CASE WHEN raw_bytes BETWEEN 0 AND 131072 THEN raw_zlib ELSE NULL END FROM raw_metadata WHERE observation_id=?1").map_err(sql_error)?;
                let row=statement.query_row([id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<Vec<u8>>>(4)?))).optional().map_err(sql_error)?;
                row.map(|(format,schema,length,hash,compressed)|{
                    let raw=compressed.map(|bytes|inflate(&bytes,length as u64)).transpose()?;
                    if raw.as_ref().is_some_and(|v|hex::encode(Sha256::digest(v.as_bytes()))!=hash){return Err(error("原始元数据摘要校验失败"));}
                    Ok(vec![vec![format,schema,(length>=0).then(||length.to_string()),raw]])
                }).transpose().map(|v|v.unwrap_or_default())
            }
        }
    }
    pub fn schema(&self, id: &str) -> Result<Vec<Vec<Option<String>>>> {
        match self {
            Self::Native(db)=>{
                let has=db.query("SELECT count(*) FROM information_schema.tables WHERE table_name='source_schemas'")?;
                if has.first().and_then(|r|r[0].as_deref())!=Some("1"){return Ok(Vec::new());}
                db.query(&format!("SELECT CAST(octet_length(schema_ipc) AS VARCHAR),CASE WHEN octet_length(schema_ipc)<=65536 THEN hex(schema_ipc) ELSE NULL END FROM source_schemas WHERE source_schema_id='{}' LIMIT 1",id.replace('\'',"''")))
            },
            Self::Online(snapshot)=>snapshot.strings(&format!("SELECT CAST(length(schema_ipc) AS TEXT),CASE WHEN length(schema_ipc)<=65536 THEN hex(schema_ipc) END FROM source_schemas WHERE source_schema_id='{}' LIMIT 1",id.replace('\'',"''")),1),
        }
    }
}
pub(crate) fn expected_metadata(source: &Source, revision: &str) -> Option<String> {
    revision
        .strip_prefix("online-v2:")
        .map(|v| format!("metadata-v2:{}:{v}", source.id))
}

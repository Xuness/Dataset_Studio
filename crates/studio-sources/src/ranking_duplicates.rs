//! Image metadata uses the newest observation; heat uses distinct post evidence.
use crate::duckdb::Session;
use studio_domain::{DuplicateHeat, Result};

const RECENCY: &str = "observed_day DESC NULLS LAST,CASE WHEN coarse_day=0 THEN observed_at_us ELSE NULL END DESC NULLS LAST,updated_at_us DESC NULLS LAST,source_priority DESC NULLS LAST,observation_id,record_id,basis";

fn observation(alias: &str) -> String {
    let fields = [
        "record_id",
        "observation_id",
        "post_id",
        "rating",
        "fav_count",
        "up_score",
        "down_score",
        "score",
        "created_at_us",
        "observed_at_us",
        "time_quality",
        "updated_at_us",
        "is_deleted",
    ];
    format!(
        "struct_pack({})",
        fields
            .map(|f| if f == "time_quality" {
                format!("{f}:=coalesce({alias}.{f},'unknown')")
            } else {
                format!("{f}:={alias}.{f}")
            })
            .join(",")
    )
}
fn clamp(column: &str) -> String {
    format!(
        "CASE WHEN p.{column} IS NULL THEN NULL ELSE CAST(greatest(-9223372036854775808::HUGEINT,least(9223372036854775807::HUGEINT,p.{column})) AS BIGINT) END"
    )
}

pub(crate) fn prepare(db: &Session, policy: DuplicateHeat) -> Result<()> {
    // Repeated query branches, asset records, and historical snapshots never
    // multiply a post's votes. First choose its latest usable complete record.
    // Keep large tag/issue payloads out of intermediate duplicate relations.
    let compact = "ordinal,basis,record_id,observation_id,post_id,rating,fav_count,up_score,down_score,score,created_at_us,observed_at_us,time_quality,updated_at_us,is_deleted,observed_day,coarse_day,source_priority";
    db.query(&format!("CREATE TEMP TABLE ranking_latest_posts AS SELECT {compact} FROM (SELECT *,row_number() OVER (PARTITION BY ordinal,CASE WHEN post_id IS NULL THEN 'record:'||record_id ELSE 'post:'||CAST(post_id AS VARCHAR) END ORDER BY {RECENCY}) AS post_position FROM ranking_observation_precision) WHERE post_position=1"))?;
    db.query(&format!("CREATE TEMP TABLE ranking_metadata AS SELECT ordinal,record_id,observation_id,basis FROM (SELECT *,row_number() OVER (PARTITION BY ordinal ORDER BY {RECENCY}) AS meta_position FROM ranking_latest_posts) WHERE meta_position=1"))?;
    // This is the same monotone heat key as the scoring operator. All fields
    // come from the winning post, including down votes and exposure dates.
    let heat = "CASE WHEN fav_count>=0 AND up_score>=0 THEN (fav_count::HUGEINT+1)*(up_score::HUGEINT+1) WHEN fav_count>=0 THEN (fav_count::HUGEINT+1)*(fav_count::HUGEINT+1) WHEN up_score>=0 THEN (up_score::HUGEINT+1)*(up_score::HUGEINT+1) ELSE NULL END";
    db.query("CREATE TEMP TABLE ranking_duplicate_ordinals AS SELECT ordinal FROM ranking_latest_posts GROUP BY ordinal HAVING count(*)>1")?;
    db.query("CREATE TEMP TABLE ranking_post_totals AS SELECT ordinal,count(*) AS post_count,min(created_at_us) AS first_created,count(DISTINCT rating)>1 AS conflict,sum(CASE WHEN fav_count>=0 THEN fav_count::HUGEINT END) AS fav,sum(CASE WHEN up_score>=0 THEN up_score::HUGEINT END) AS up,sum(abs(down_score::HUGEINT)) AS down,sum(score::HUGEINT) AS net,bool_or(fav_count IS NULL OR fav_count<0 OR up_score IS NULL OR up_score<0 OR down_score IS NULL OR score IS NULL) AS partial FROM ranking_latest_posts WHERE ordinal IN (SELECT ordinal FROM ranking_duplicate_ordinals) GROUP BY ordinal")?;
    db.query(&format!("CREATE TEMP TABLE ranking_heat_posts AS SELECT *,row_number() OVER (PARTITION BY ordinal ORDER BY {heat} DESC NULLS LAST,score DESC NULLS LAST,{RECENCY}) AS heat_position FROM ranking_latest_posts WHERE ordinal IN (SELECT ordinal FROM ranking_post_totals)"))?;
    let sum = policy == DuplicateHeat::Sum;
    let shown = if sum { 64 } else { 1 };
    db.query(&format!("CREATE TEMP TABLE ranking_heat_evidence AS SELECT ordinal,list({} ORDER BY heat_position) AS heat FROM ranking_heat_posts h WHERE heat_position<={shown} GROUP BY ordinal", observation("h")))?;
    let policy_name = if sum { "sum" } else { "highest" };
    let merged_numeric = if sum {
        format!(
            "{} AS fav_count,{} AS up_score,{} AS down_score,{} AS score,CASE WHEN p.post_count>1 THEN p.first_created ELSE h.created_at_us END AS created_at_us,CASE WHEN p.post_count>1 THEN NULL ELSE h.observed_at_us END AS observed_at_us,CASE WHEN p.post_count>1 THEN 'aggregate' ELSE h.time_quality END AS time_quality",
            clamp("fav"),
            clamp("up"),
            clamp("down"),
            clamp("net")
        )
    } else {
        "h.fav_count,h.up_score,h.down_score,h.score,h.created_at_us,h.observed_at_us,h.time_quality".into()
    };
    db.query(&format!("CREATE TEMP TABLE ranking_heat_values AS SELECT h.ordinal,{merged_numeric} FROM ranking_heat_posts h JOIN ranking_post_totals p ON p.ordinal=h.ordinal WHERE h.heat_position=1"))?;
    let numeric = [
        "fav_count",
        "up_score",
        "down_score",
        "score",
        "created_at_us",
        "observed_at_us",
        "time_quality",
    ]
    .map(|f| format!("CASE WHEN p.ordinal IS NULL THEN c.{f} ELSE h.{f} END AS {f}"))
    .join(",");
    let bounded = "coalesce(p.fav>9223372036854775807::HUGEINT OR p.up>9223372036854775807::HUGEINT OR p.down>9223372036854775807::HUGEINT OR p.net>9223372036854775807::HUGEINT OR p.net< -9223372036854775808::HUGEINT,false)";
    db.query(&format!("CREATE TEMP TABLE ranking_chosen AS SELECT c.* EXCLUDE(fav_count,up_score,down_score,score,created_at_us,observed_at_us,time_quality,rating_conflict),{numeric},coalesce(p.conflict,false) AS rating_conflict,CASE WHEN p.post_count>1 THEN to_json(struct_pack(policy:='{policy_name}',metadata:={},heat:=e.heat,post_count:=p.post_count,omitted_posts:=CASE WHEN {sum} THEN greatest(0,p.post_count-{shown}) ELSE 0 END,partial_counts:={sum} AND p.partial,counts_clamped:={sum} AND {bounded}))::VARCHAR ELSE NULL END AS evidence_json FROM ranking_metadata m JOIN ranking_observation_precision c USING(ordinal,record_id,observation_id,basis) LEFT JOIN ranking_heat_values h ON h.ordinal=c.ordinal LEFT JOIN ranking_post_totals p ON p.ordinal=c.ordinal LEFT JOIN ranking_heat_evidence e ON e.ordinal=c.ordinal",observation("c")))?;
    db.query("DROP TABLE ranking_latest_posts; DROP TABLE ranking_metadata; DROP TABLE ranking_duplicate_ordinals; DROP TABLE ranking_post_totals; DROP TABLE ranking_heat_posts; DROP TABLE ranking_heat_evidence; DROP TABLE ranking_heat_values")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn duplicate_posts_use_latest_rating_and_do_not_sum_history() {
        for policy in [DuplicateHeat::Highest, DuplicateHeat::Sum] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-runs");
            fs::create_dir_all(&root).unwrap();
            let temp = tempfile::Builder::new()
                .prefix("duplicate-heat-")
                .tempdir_in(&root)
                .unwrap();
            let db = Session::fixture(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/duckdb/duckdb.dll"),
                &temp.path().join("case.duckdb"),
            )
            .unwrap();
            db.query("CREATE TABLE ranking_observation_precision(ordinal BIGINT,basis BIGINT,record_id VARCHAR,observation_id VARCHAR,post_id BIGINT,rating VARCHAR,fav_count BIGINT,up_score BIGINT,down_score BIGINT,score BIGINT,created_at_us BIGINT,observed_at_us BIGINT,time_quality VARCHAR,updated_at_us BIGINT,is_deleted BOOLEAN,observed_day BIGINT,coarse_day BIGINT,source_priority BIGINT,rating_conflict BOOLEAN); INSERT INTO ranking_observation_precision VALUES (0,0,'a','old',4010829,'g',9999,9999,0,9999,1,10,'exact',10,false,1,0,1,true),(0,0,'a','new',4010829,'g',50,32,0,32,1,20,'date_only',20,false,2,1,1,true),(0,1,'a','new',4010829,'g',50,32,0,32,1,20,'date_only',20,false,2,1,1,true),(0,0,'b','last',4529147,'s',12,7,0,7,2,20,'date_only',30,true,2,1,1,true),(1,0,'c','single',5,'q',4,3,0,3,1,20,'exact',20,false,2,0,1,false)").unwrap();
            db.query("INSERT INTO ranking_observation_precision VALUES (2,0,'huge','huge',10,'g',9223372036854775807,9223372036854775807,-9223372036854775808,9223372036854775807,1,20,'exact',20,false,2,0,1,true),(2,0,'partial','partial',11,'s',1,NULL,NULL,-1,1,20,'exact',30,false,2,0,1,true); INSERT INTO ranking_observation_precision SELECT 3,0,'many'||CAST(i AS VARCHAR),'many'||CAST(i AS VARCHAR),100+i,'g',1,1,0,1,1,20,'exact',i,false,2,0,1,false FROM range(70) t(i)").unwrap();
            prepare(&db, policy).unwrap();
            let row = db.query("SELECT to_json(struct_pack(post_id:=post_id,rating:=rating,fav:=fav_count,up:=up_score,score:=score,created:=created_at_us,quality:=time_quality,evidence:=CAST(evidence_json AS JSON))) FROM ranking_chosen WHERE ordinal=0").unwrap();
            assert_eq!(row.len(), 1);
            let value: serde_json::Value =
                serde_json::from_str(row[0][0].as_ref().unwrap()).unwrap();
            assert_eq!(value["post_id"], 4529147);
            assert_eq!(value["rating"], "s");
            assert_eq!(value["evidence"]["post_count"], 2);
            assert_eq!(value["evidence"]["metadata"]["fav_count"], 12);
            assert_eq!(value["evidence"]["heat"][0]["post_id"], 4010829);
            assert_eq!(value["created"], 1);
            if policy == DuplicateHeat::Highest {
                assert_eq!(value["fav"], 50);
                assert_eq!(value["score"], 32);
            } else {
                assert_eq!(value["fav"], 62);
                assert_eq!(value["score"], 39);
                assert_eq!(value["quality"], "aggregate");
            }
            let single = db.query("SELECT CAST(fav_count AS VARCHAR),evidence_json FROM ranking_chosen WHERE ordinal=1").unwrap();
            assert_eq!(single[0], vec![Some("4".into()), None]);
            let large=db.query("SELECT evidence_json FROM ranking_chosen WHERE ordinal IN (2,3) ORDER BY ordinal").unwrap();
            let evidence = large
                .iter()
                .map(|r| serde_json::from_str::<serde_json::Value>(r[0].as_ref().unwrap()).unwrap())
                .collect::<Vec<_>>();
            if policy == DuplicateHeat::Sum {
                assert_eq!(evidence[0]["counts_clamped"], true);
                assert_eq!(evidence[0]["partial_counts"], true);
                assert_eq!(evidence[1]["heat"].as_array().unwrap().len(), 64);
                assert_eq!(evidence[1]["omitted_posts"], 6);
            } else {
                assert_eq!(evidence[1]["heat"].as_array().unwrap().len(), 1);
            }
        }
    }
}

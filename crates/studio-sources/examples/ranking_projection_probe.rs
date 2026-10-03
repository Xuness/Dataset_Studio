//! Bounded, read-only real-lake projection timing. It never submits a job.
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
use studio_application::QueryAdapter;
use studio_domain::*;
use studio_sources::{QueryReader, RankingReader};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("studio_sources=debug")
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Err(Error::invalid(
            "Usage: ranking_projection_probe <index-root> <media-root> <run.json> [rows<=262144] [no-dimensions]",
        ));
    }
    let root = PathBuf::from(&args[1]);
    let count: usize = args
        .get(4)
        .map(|n| n.parse().map_err(Error::io))
        .transpose()?
        .unwrap_or(32768);
    if count == 0 || count > 262144 {
        return Err(Error::invalid("Probe row limit exceeded"));
    }
    let pointer: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("ONLINE.json")).map_err(Error::io)?)
            .map_err(Error::io)?;
    let source = Source {
        id: pointer["library_id"]
            .as_str()
            .ok_or_else(|| Error::invalid("Missing lake identity"))?
            .into(),
        name: "Projection probe".into(),
        kind: "danbooru".into(),
        index_root: Some(root.clone()),
        media_root: Some(PathBuf::from(&args[2])),
    };
    let run: OperatorRun =
        serde_json::from_slice(&std::fs::read(&args[3]).map_err(Error::io)?).map_err(Error::io)?;
    let mut parameters: RankingParameters =
        serde_json::from_value(run.parameters).map_err(Error::io)?;
    if args.get(5).is_some_and(|s| s == "no-dimensions") {
        parameters.minimum_stored_side = None;
    }
    let version = QueryReader::default().read_version(&source, true)?;
    let sequence: i64 = version
        .analysis_sequence
        .as_deref()
        .ok_or_else(|| Error::invalid("No sequence"))?
        .parse()
        .map_err(Error::io)?;
    let db = rusqlite::Connection::open_with_flags(
        root.join(
            pointer["file"]
                .as_str()
                .ok_or_else(|| Error::invalid("No online file"))?,
        ),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(Error::io)?;
    let keys = db
        .prepare("SELECT sha256 FROM objects WHERE first_seq<=?1 ORDER BY sha256 LIMIT ?2")
        .map_err(Error::io)?
        .query_map(rusqlite::params![sequence, count as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(Error::io)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::io)?;
    drop(db);
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-runs/ranking-300s/projection-scratch");
    let reader = RankingReader::configured(scratch, 16 << 30);
    let mut rows = 0usize;
    let mut dimensions = std::collections::BTreeMap::<String, usize>::new();
    let started = Instant::now();
    reader.project(
        &source,
        &version,
        &[],
        &parameters,
        Arc::new(AtomicBool::new(false)),
        &mut |append| {
            for (ordinal, key) in keys.iter().enumerate() {
                append(ordinal as u64, key, 0)?;
            }
            Ok(())
        },
        &mut |page| {
            rows += page.len();
            for row in page {
                *dimensions.entry(row.dimension_basis.clone()).or_default() += 1;
            }
            Ok(())
        },
    )?;
    println!(
        "{}",
        serde_json::json!({"rows":rows,"seconds":started.elapsed().as_secs_f64(),"dimensions":dimensions})
    );
    Ok(())
}

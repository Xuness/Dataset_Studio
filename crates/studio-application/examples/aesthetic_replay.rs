//! Reproducible CPU/memory probe: synthetic streamed evidence, no DB/media/network.
use studio_application::aesthetic_analysis::{AestheticReplaySource, estimator::replay};
use studio_domain::{AssetKey, Result, aesthetic::AestheticCandidate, aesthetic_analysis::*};
struct Synthetic {
    count: u64,
    rounds: u64,
}
impl AestheticReplaySource for Synthetic {
    fn replay_candidates(&self, _: &str, after: Option<u64>) -> Result<Vec<AestheticCandidate>> {
        let start = after.map_or(0, |n| n + 1);
        Ok((start..(start + 64).min(self.count))
            .map(|ordinal| AestheticCandidate {
                ordinal,
                key: AssetKey {
                    source_id: "synthetic".into(),
                    asset_id: format!("{ordinal:064x}"),
                },
                rating: "g".into(),
                year: Some(2000 + (ordinal % 20) as i32),
                basis: "synthetic".into(),
                content_version: "v1".into(),
                bytes: 0,
                exposures: 0,
                protected: false,
            })
            .collect())
    }
    fn replay_observations(
        &self,
        _: &AestheticAnalysisInput,
        after: u64,
    ) -> Result<Vec<AestheticReplayObservation>> {
        let batches = self.count / 16;
        Ok((after..(after + 64).min(batches * self.rounds))
            .map(|b| {
                let round = b / batches;
                let stride = [1, 17, 31, 49][round as usize % 4];
                let mut ids = (0..16)
                    .map(|slot| {
                        ((((b % batches) * 16 + slot) * stride + round * 19) % self.count) as u32
                    })
                    .collect::<Vec<_>>();
                ids.sort_unstable();
                let elite = if ids.contains(&0) { vec![0] } else { vec![] };
                AestheticReplayObservation {
                    batch: b + 1,
                    rating: "g".into(),
                    tiers: ids.chunks(3).map(|s| s.to_vec()).collect(),
                    elite,
                    unjudgeable: vec![],
                }
            })
            .collect())
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let count = args
        .first()
        .map(|v| v.parse::<u64>())
        .transpose()
        .map_err(studio_domain::Error::io)?
        .unwrap_or(10_000);
    let kind = args.get(1).cloned().unwrap_or_else(|| "davidson_v1".into());
    let iterations = args
        .get(2)
        .map(|v| v.parse::<u32>())
        .transpose()
        .map_err(studio_domain::Error::io)?
        .unwrap_or(32);
    if count % 16 != 0 || count == 0 {
        return Err(studio_domain::Error::invalid("候选数需为 16 的正整数倍"));
    }
    for stride in [7, 17, 31] {
        if count % stride == 0 {
            return Err(studio_domain::Error::invalid(
                "此合成排列要求候选数与 17、31、49 互质",
            ));
        }
    }
    let source = Synthetic { count, rounds: 4 };
    let stage_id = studio_domain::new_id();
    let input = AestheticAnalysisInput {
        stage_id: stage_id.clone(),
        stage_config_hash: "synthetic-v1".into(),
        evidence_watermark: count / 4,
        observations: count / 4,
        candidates: count,
        review_watermark: 0,
    };
    let config = AestheticFit {
        stage_id,
        estimator: AestheticEstimator {
            kind,
            iterations,
            regularization: 0.1,
            tie_strength: 1.0,
        },
        stability_seed: Some(17),
    };
    let start = std::time::Instant::now();
    let mut emitted = 0u64;
    let mut exposures = 0u64;
    let summary = replay(
        &source,
        &input,
        &config,
        &|| Ok(()),
        &mut |_, _, _| Ok(()),
        &mut |rows| {
            emitted += rows.len() as u64;
            exposures += rows.iter().map(|r| u64::from(r.exposures)).sum::<u64>();
            Ok(())
        },
    )?;
    assert_eq!(emitted, count);
    assert_eq!(exposures, count * 4);
    println!(
        "{}",
        serde_json::json!({"synthetic":true,"database":false,"network":false,"images":false,"candidates":count,"batches":input.observations,"emitted":emitted,"valid_exposures":exposures,"seconds":start.elapsed().as_secs_f64(),"summary":summary})
    );
    Ok(())
}

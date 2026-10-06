pub mod grouping;
use std::collections::BTreeSet;
use studio_domain::{Error, Result, aesthetic::*, llm::*};
mod execution;
mod lifecycle;
pub use execution::*;
pub mod sampling;
pub use lifecycle::*;

/// Two regrouping opportunities before an unseen candidate requires review.
pub const UNJUDGEABLE_REVIEW_THRESHOLD: u32 = 3;

/// Storage implements this port; accepted observations are the replay source of truth.
pub trait AestheticRepository: Send + Sync {
    fn stage(&self, id: &str) -> Result<AestheticStage>;
    fn stages(&self, after: Option<&str>, limit: usize) -> Result<Vec<AestheticStage>>;
    fn batches(&self, id: &str, after: u64, limit: usize) -> Result<Vec<AestheticBatch>>;
    fn attempts(&self, id: &str, batch: u64) -> Result<Vec<AestheticAttempt>>;
    fn receive(&self, id: &str, attempt: &str, receipt: AestheticReceipt) -> Result<()>;
    fn parse_received(&self, id: &str) -> Result<u64>;
}

pub fn validate_create(value: &AestheticCreate) -> Result<()> {
    if let Some(policy) = &value.execution_policy {
        validate_execution_policy(policy)?;
        if policy.concurrency != value.concurrency {
            return Err(Error::invalid("执行策略并发与阶段并发不一致"));
        }
    }
    if value
        .budget_mode
        .as_ref()
        .is_some_and(|v| !matches!(v.as_str(), "complete" | "trial"))
    {
        return Err(Error::invalid("预算方式必须为完整曝光或小预算试跑"));
    }
    for id in [
        &value.idempotency_key,
        &value.collection_id,
        &value.model_id,
        &value.system_prompt_id,
    ] {
        studio_domain::validate_id(id)?;
    }
    studio_domain::validate_name(&value.name)?;
    if value
        .max_request_mib
        .is_some_and(|v| !(8..=48).contains(&v))
    {
        return Err(Error::invalid("请求体预算须为 8–48 MiB"));
    }
    if !(1..=32).contains(&value.exposures)
        || !(1..=AESTHETIC_MAX_CONCURRENCY).contains(&value.concurrency)
        || !(1..=10_000_000).contains(&value.max_calls)
    {
        return Err(Error::invalid(
            "曝光次数 1–32，并发 1–1024，调用上限 1–10000000",
        ));
    }
    if value
        .expected_input_version
        .as_ref()
        .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::invalid("预检输入版本必须为 SHA-256"));
    }
    if let Some(policy) = &value.sampling {
        sampling::validate(policy, 0)?;
        if policy.min_exposures != value.exposures {
            return Err(Error::invalid("最低曝光与阶段曝光字段不一致"));
        }
    }
    Ok(())
}

pub const OUTPUT_INSTRUCTIONS: &str = r#"
每个图片 ID 文本只标识紧随其后的那一张图片。严格原样复制 ID，禁止使用文件名、附件编号或自行编号。
返回一个 JSON 对象，不使用 Markdown：
{"schema_version":1,"tiers":[["img01","img02"],["img03"]],"elite_candidates":[],"unjudgeable":[]}
tiers 从审美高到低排列。同梯队表示在本次审美标准下没有有意义的差异，而非省略内部排序。
所有输入图片 ID 必须在 tiers 或 unjudgeable 中恰好出现一次，不允许未知、遗漏或重复 ID。
无法判断的图片放入 unjudgeable，格式 {"id":"img01","reason":"具体原因"}，不把它当作低质量。
elite_candidates 仅提名达到 System Prompt 顶级标准的图片，允许为空，不能强制选本批第一名。
顶级提名仍需参与 tiers。使用给出的短 ID，不推测文件身份或元数据。
"#;

pub fn parse_observation(
    receipt: &AestheticReceipt,
    members: &[AestheticMember],
) -> Result<AestheticObservation> {
    if receipt.outputs.len() != 1 {
        return Err(Error::invalid("评审必须返回一个完整结果"));
    }
    let output = &receipt.outputs[0];
    if !matches!(
        output.finish_reason.as_deref(),
        Some("stop" | "STOP" | "completed" | "end_turn")
    ) {
        return Err(Error::invalid("评审未正常结束，不能接受截断或未知结束状态"));
    }
    let mut text = String::new();
    for block in &output.content {
        match block {
            LlmContent::Text { text: part } => text.push_str(part),
            LlmContent::Reasoning { .. } => {}
            _ => return Err(Error::invalid("评审包含拒绝或非文本输出")),
        }
    }
    if text.len() > 256 * 1024 {
        return Err(Error::invalid("评审 JSON 超出 256 KiB"));
    }
    let value: AestheticObservation =
        serde_json::from_str(&text).map_err(|_| Error::invalid("评审不是约定的 JSON 对象"))?;
    validate_observation(&value, members)?;
    // Keep structural validation compatible with immutable historical evidence.
    // New receipts must actually compare at least two images before acceptance.
    if value.tiers.iter().map(Vec::len).sum::<usize>() < 2 {
        return Err(Error::new(
            "EVALUATION_NO_COMPARABLE_EVIDENCE",
            "本批可比较图片不足 2 张，未产生有效比较证据，可按执行策略重试",
        ));
    }
    Ok(value)
}

pub fn validate_observation(
    value: &AestheticObservation,
    members: &[AestheticMember],
) -> Result<()> {
    let expected: BTreeSet<_> = members.iter().map(|m| m.label.as_str()).collect();
    if value.schema_version != AESTHETIC_VERSION
        || members.is_empty()
        || members.len() > 16
        || expected.len() != members.len()
        || members
            .iter()
            .any(|m| m.candidate.rating != members[0].candidate.rating)
    {
        return Err(Error::invalid("评审版本、批次成员或 Rating 无效"));
    }
    let mut seen = BTreeSet::new();
    for tier in &value.tiers {
        if tier.is_empty() {
            return Err(Error::invalid("梯队不可为空"));
        }
        for id in tier {
            if !expected.contains(id.as_str()) || !seen.insert(id.as_str()) {
                return Err(Error::invalid("梯队存在未知或重复 ID"));
            }
        }
    }
    let mut elite = BTreeSet::new();
    for id in &value.elite_candidates {
        if !seen.contains(id.as_str()) || !elite.insert(id.as_str()) {
            return Err(Error::invalid("顶级候选必须是不重复的已评判图片"));
        }
    }
    for item in &value.unjudgeable {
        if item.reason.trim().is_empty()
            || item.reason.len() > 1000
            || !expected.contains(item.id.as_str())
            || !seen.insert(item.id.as_str())
        {
            return Err(Error::invalid("不可评判图片的 ID 或原因无效"));
        }
    }
    if seen != expected {
        return Err(Error::invalid("评审遗漏了输入图片"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn members() -> Vec<AestheticMember> {
        (0..2)
            .map(|n| AestheticMember {
                label: format!("img{n}"),
                candidate: AestheticCandidate {
                    ordinal: n,
                    key: studio_domain::AssetKey {
                        source_id: "source".into(),
                        asset_id: n.to_string(),
                    },
                    rating: "g".into(),
                    year: None,
                    basis: "test".into(),
                    content_version: "test".into(),
                    bytes: 1,
                    exposures: 0,
                    protected: false,
                    disposition: Default::default(),
                    disposition_reason: None,
                    blocked: false,
                    blocking_batch: None,
                },
                image_sha256: None,
            })
            .collect()
    }
    #[test]
    fn rejects_duplicate_missing_foreign_ids_and_elite_outside_judged_members() {
        let members = members();
        let valid = AestheticObservation {
            schema_version: 1,
            tiers: vec![vec!["img0".into(), "img1".into()]],
            elite_candidates: vec!["img0".into()],
            unjudgeable: vec![],
        };
        validate_observation(&valid, &members).unwrap();
        for tiers in [
            vec![vec!["img0".into(), "img0".into()]],
            vec![vec!["img0".into()]],
            vec![vec!["img0".into(), "foreign".into()]],
            vec![vec![], vec!["img0".into(), "img1".into()]],
        ] {
            let value = AestheticObservation {
                tiers,
                ..valid.clone()
            };
            assert!(validate_observation(&value, &members).is_err());
        }
        let mut abstain = valid.clone();
        abstain.tiers = vec![vec!["img1".into()]];
        abstain.unjudgeable = vec![AestheticUnjudgeable {
            id: "img0".into(),
            reason: "cannot see".into(),
        }];
        assert!(validate_observation(&abstain, &members).is_err());
        let mut mixed = members.clone();
        mixed[1].candidate.rating = "e".into();
        assert!(validate_observation(&valid, &mixed).is_err());
    }
    #[test]
    fn refuses_truncated_and_refusal_responses_even_when_text_looks_valid() {
        let mut value=AestheticReceipt{provider_request_id:None,response_id:None,model:None,usage:Default::default(),outputs:vec![LlmOutput{index:0,finish_reason:Some("length".into()),content:vec![LlmContent::Text{text:r#"{"schema_version":1,"tiers":[["img0","img1"]],"elite_candidates":[],"unjudgeable":[]}"#.into()}]}]};
        assert!(parse_observation(&value, &members()).is_err());
        value.outputs[0].finish_reason = Some("stop".into());
        parse_observation(&value, &members()).unwrap();
        value.outputs[0].content.push(LlmContent::Refusal {
            text: "refused".into(),
        });
        assert!(parse_observation(&value, &members()).is_err());
    }
    #[test]
    fn output_contract_retains_abstentions_but_requires_comparative_evidence() {
        let text = r#"{"schema_version":1,"tiers":[],"elite_candidates":[],"unjudgeable":[{"id":"img0","reason":"无法辨认"},{"id":"img1","reason":"无法辨认"}]}"#;
        let mut receipt = AestheticReceipt {
            provider_request_id: None,
            response_id: None,
            model: None,
            usage: Default::default(),
            outputs: vec![LlmOutput {
                index: 0,
                finish_reason: Some("stop".into()),
                content: vec![LlmContent::Text { text: text.into() }],
            }],
        };
        validate_observation(&serde_json::from_str(text).unwrap(), &members()).unwrap();
        assert_eq!(
            parse_observation(&receipt, &members()).unwrap_err().code,
            "EVALUATION_NO_COMPARABLE_EVIDENCE"
        );
        receipt.outputs[0].content = vec![LlmContent::Text {
            text: r#"{"schema_version":1,"tiers":[["img0"]],"elite_candidates":["img0"],"unjudgeable":[{"id":"img1","reason":"无法辨认"}]}"#.into(),
        }];
        assert_eq!(
            parse_observation(&receipt, &members()).unwrap_err().code,
            "EVALUATION_NO_COMPARABLE_EVIDENCE"
        );
        for invalid in [
            text.replacen("{", r#"{"score":100,"#, 1),
            text.replace(r#""reason":"无法辨认""#, r#""reason":"" "#),
            text.replace("无法辨认", &"模".repeat(334)),
            format!("\x60\x60\x60json\n{text}\n\x60\x60\x60"),
        ] {
            receipt.outputs[0].content = vec![LlmContent::Text { text: invalid }];
            assert!(parse_observation(&receipt, &members()).is_err());
        }
    }
}

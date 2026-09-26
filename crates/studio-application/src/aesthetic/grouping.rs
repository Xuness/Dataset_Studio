//! Grouping policy is independent of the database that supplies these aggregate facts.
use studio_domain::*;

pub struct OriginGroupFacts {
    pub distinct_ratings: u64,
    pub known_ratings: u64,
    pub record_count: u64,
    pub rating: Option<String>,
    pub earliest_post_year: Option<i32>,
    pub record_example: Option<String>,
    pub observation_example: Option<String>,
}
pub fn origin_group(
    facts: OriginGroupFacts,
    version: &str,
) -> Result<(String, Option<i32>, String)> {
    if facts.known_ratings > facts.record_count || facts.distinct_ratings > facts.known_ratings {
        return Err(Error::new("SOURCE_FORMAT_ERROR", "来源分组计数不一致"));
    }
    let rating = if facts.distinct_ratings > 1 {
        "conflict".into()
    } else if facts.known_ratings < facts.record_count {
        "unknown".into()
    } else {
        facts.rating.unwrap_or_else(|| "unknown".into())
    };
    let basis=serde_json::json!({"rule":"origin_rating_agreement_min_post_created_year_v1","version":version,"record_example":facts.record_example,"observation_example":facts.observation_example,"record_count":facts.record_count.to_string()}).to_string();
    Ok((rating, facts.earliest_post_year, basis))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_and_conflicting_ratings_do_not_become_known() {
        for (distinct, known, expected) in [(1, 1, "unknown"), (2, 2, "conflict"), (1, 2, "g")] {
            let result = origin_group(
                OriginGroupFacts {
                    distinct_ratings: distinct,
                    known_ratings: known,
                    record_count: 2,
                    rating: Some("g".into()),
                    earliest_post_year: None,
                    record_example: None,
                    observation_example: None,
                },
                "v1",
            )
            .unwrap();
            assert_eq!(result.0, expected);
            assert_eq!(result.1, None);
        }
    }
}

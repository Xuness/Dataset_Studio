//! Read two optional fields without allocating the unrelated source JSON tree.
use serde::{
    Deserialize, Serialize,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;

#[derive(Default, Serialize)]
pub(super) struct Dimensions {
    raw_stored_width: Option<Value>,
    raw_stored_height: Option<Value>,
}

#[derive(Deserialize)]
#[serde(field_identifier)]
enum Field {
    #[serde(rename = "raw_stored_width")]
    Width,
    #[serde(rename = "raw_stored_height")]
    Height,
    #[serde(other)]
    Other,
}

impl<'de> Deserialize<'de> for Dimensions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Dimensions;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON metadata value")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Dimensions, M::Error> {
                let mut dimensions = Dimensions::default();
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        // Preserve Value::get's last-key-wins behavior, and its
                        // original number/string/null types for the SQL cast.
                        Field::Width => dimensions.raw_stored_width = Some(map.next_value()?),
                        Field::Height => dimensions.raw_stored_height = Some(map.next_value()?),
                        Field::Other => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(dimensions)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Dimensions, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Dimensions::default())
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Dimensions, E> {
                Ok(Dimensions::default())
            }
        }
        deserializer.deserialize_any(Fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selective_fields_preserve_original_json_types_and_duplicate_keys() {
        for text in [
            r#"{"raw_stored_width":640,"raw_stored_height":480,"unrelated":{"many":[1,2,3]}}"#,
            r#"{"raw_stored_width":null,"raw_stored_height":"2048","id":123456789012345678901}"#,
            r#"{"raw_stored_width":12,"raw_stored_width":42,"raw_stored_height":1.25}"#,
            r#"{"raw_stored_width":true,"raw_stored_height":[1,2],"nested":{"raw_stored_width":888}}"#,
            "{}",
            "null",
            "[]",
            "true",
            "42",
            "-3",
            "3.5",
            r#""metadata""#,
        ] {
            let original: Value = serde_json::from_str(text).unwrap();
            let expected = serde_json::json!({"raw_stored_width": original.get("raw_stored_width"), "raw_stored_height": original.get("raw_stored_height")});
            let actual: Dimensions = serde_json::from_str(text).unwrap();
            assert_eq!(serde_json::to_value(actual).unwrap(), expected);
        }
        for text in [
            r#"{"raw_stored_width":}"#,
            r#"{"ignored":[1,]}"#,
            "{} trailing",
        ] {
            assert!(serde_json::from_str::<Dimensions>(text).is_err());
        }
    }
}

//! Map text-key bounds to the SHA-256 BLOB index without casting that index's
//! entire contents to text. Added members can have opaque adapter-owned IDs.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HashBound {
    All,
    Empty,
    Compare(&'static str, Vec<u8>),
}
const DIGITS: &[u8] = b"0123456789abcdef";
fn increment(mut prefix: Vec<u8>) -> Option<Vec<u8>> {
    while let Some(last) = prefix.pop() {
        if last != b'f' {
            let index = DIGITS.iter().position(|&b| b == last).expect("hex prefix");
            prefix.push(DIGITS[index + 1]);
            prefix.resize(64, b'0');
            return Some(prefix);
        }
    }
    None
}
fn ceiling(value: &str) -> Option<Vec<u8>> {
    let mut prefix = Vec::with_capacity(64);
    for index in 0..64 {
        let Some(&next) = value.as_bytes().get(index) else {
            prefix.resize(64, b'0');
            return Some(prefix);
        };
        if DIGITS.contains(&next) {
            prefix.push(next);
        } else if let Some(&higher) = DIGITS.iter().find(|&&b| b > next) {
            prefix.push(higher);
            prefix.resize(64, b'0');
            return Some(prefix);
        } else {
            return increment(prefix);
        }
    }
    if value.len() == 64 {
        Some(prefix)
    } else {
        increment(prefix)
    }
}
pub(super) fn hash_bound(operator: &str, value: &str) -> HashBound {
    let exact = crate::ranking_tables::canonical_asset_id(value);
    if operator == "=" && !exact {
        return HashBound::Empty;
    }
    let Some(ceil) = ceiling(value) else {
        return if matches!(operator, "<" | "<=") {
            HashBound::All
        } else {
            HashBound::Empty
        };
    };
    let operator = match operator {
        "=" => "=",
        ">" if exact => ">",
        ">" | ">=" => ">=",
        "<=" if exact => "<=",
        "<" | "<=" => "<",
        _ => return HashBound::Empty,
    };
    HashBound::Compare(
        operator,
        hex::decode(ceil).expect("canonical hexadecimal bound"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blob_ranges_match_text_order_for_canonical_opaque_and_sentinel_bounds() {
        let mut keys = (0..256).map(|n| format!("{n:064x}")).collect::<Vec<_>>();
        keys.extend(["a".repeat(64), "f".repeat(64), "c0".repeat(32)]);
        let mut bounds = vec![
            "".into(),
            "sample-0001".into(),
            "参考图".into(),
            "f".repeat(65),
            "f".repeat(63),
            "A".repeat(64),
            "a".into(),
            "a-".into(),
            "c0sample".into(),
            "0".repeat(65),
        ];
        for key in &keys {
            bounds.extend([key.clone(), format!("{key}x"), format!("{}g", &key[..63])]);
        }
        for bound in bounds {
            for op in ["=", ">", ">=", "<", "<="] {
                let actual = hash_bound(op, &bound);
                for key in &keys {
                    let expected = match op {
                        "=" => key == &bound,
                        ">" => key > &bound,
                        ">=" => key >= &bound,
                        "<" => key < &bound,
                        _ => key <= &bound,
                    };
                    let matched = match &actual {
                        HashBound::All => true,
                        HashBound::Empty => false,
                        HashBound::Compare(op, value) => {
                            let key = hex::decode(key).unwrap();
                            match *op {
                                "=" => key == *value,
                                ">" => key > *value,
                                ">=" => key >= *value,
                                "<" => key < *value,
                                _ => key <= *value,
                            }
                        }
                    };
                    assert_eq!(matched, expected, "{key} {op} {bound}");
                }
            }
        }
    }
}

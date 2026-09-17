const FLOAT_SENTINEL: &str = "\0crossline-json-f64:";
const FLOAT_SENTINEL_JSON: &str = "\\u0000crossline-json-f64:";

pub(super) fn parse_json_with_exact_floats(input: &str) -> Result<serde_json::Value, String> {
    let validation = serde_json::from_str::<serde_json::Value>(input)
        .map_err(|error| format!("invalid JSON payload: {error}"))?;
    if contains_float_sentinel(&validation) {
        return Err("JSON payload contains reserved float sentinel".to_owned());
    }

    let bytes = input.as_bytes();
    let mut marked = String::with_capacity(input.len());
    let mut cursor = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            index = string_end(bytes, index)?;
            continue;
        }
        if bytes[index] != b'-' && !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let end = number_end(bytes, index)?;
        let token = &input[index..end];
        if token.contains(['.', 'e', 'E']) {
            marked.push_str(&input[cursor..index]);
            marked.push('"');
            marked.push_str(FLOAT_SENTINEL_JSON);
            marked.push_str(token);
            marked.push('"');
            cursor = end;
        }
        index = end;
    }
    marked.push_str(&input[cursor..]);

    let mut payload = serde_json::from_str::<serde_json::Value>(&marked)
        .map_err(|error| format!("marked JSON payload decode failed: {error}"))?;
    restore_float_tokens(&mut payload)?;
    Ok(payload)
}

fn contains_float_sentinel(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => value.starts_with(FLOAT_SENTINEL),
        serde_json::Value::Array(values) => values.iter().any(contains_float_sentinel),
        serde_json::Value::Object(values) => values.values().any(contains_float_sentinel),
        _ => false,
    }
}

fn restore_float_tokens(value: &mut serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::String(text) if text.starts_with(FLOAT_SENTINEL) => {
            let token = text.trim_start_matches(FLOAT_SENTINEL);
            let parsed = token
                .parse::<f64>()
                .map_err(|error| format!("JSON float token parse failed: {error}"))?;
            let number = serde_json::Number::from_f64(parsed)
                .ok_or_else(|| "JSON float token is not finite".to_owned())?;
            *value = serde_json::Value::Number(number);
            Ok(())
        }
        serde_json::Value::Array(values) => {
            for value in values {
                restore_float_tokens(value)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                restore_float_tokens(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn string_end(bytes: &[u8], start: usize) -> Result<usize, String> {
    let mut index = start.saturating_add(1);
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'"' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err("unterminated JSON string".to_owned())
}

fn number_end(bytes: &[u8], start: usize) -> Result<usize, String> {
    let mut index = start;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }
    match bytes.get(index) {
        Some(b'0') => index += 1,
        Some(b'1'..=b'9') => {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return Err("invalid JSON number integer part".to_owned()),
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return Err("invalid JSON number fraction".to_owned());
        }
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return Err("invalid JSON number exponent".to_owned());
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::parse_json_with_exact_floats;

    #[test]
    fn normalizes_postgres_expanded_float_without_touching_other_tokens() {
        let input = concat!(
            r#"{"value":-0.00000000000011368683772161603,"#,
            r#""label":"-0.00000000000011368683772161603","count":9007199254740993}"#,
        );

        let payload = parse_json_with_exact_floats(input).expect("exact payload");

        assert_eq!(
            payload["value"].as_f64(),
            Some(-1.136_868_377_216_160_3e-13)
        );
        assert_eq!(
            payload["label"].as_str(),
            Some("-0.00000000000011368683772161603")
        );
        assert_eq!(payload["count"].as_u64(), Some(9_007_199_254_740_993));
    }

    #[test]
    fn normalizes_nested_fraction_and_exponent_tokens() {
        let input = r#"{"rows":[1.2300,1e3,-0.0],"escaped":"\"2.5000"}"#;

        let payload = parse_json_with_exact_floats(input).expect("exact payload");

        assert_eq!(payload["rows"][0].as_f64(), Some(1.23));
        assert_eq!(payload["rows"][1].as_f64(), Some(1000.0));
        assert_eq!(
            payload["rows"][2].as_f64().map(f64::is_sign_negative),
            Some(true)
        );
        assert_eq!(payload["escaped"].as_str(), Some("\"2.5000"));
    }
}

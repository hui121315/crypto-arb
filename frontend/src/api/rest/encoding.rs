//! REST 路径段与查询值的 RFC 3986 组件编码。

pub(super) fn encode_path_segment(value: &str) -> String {
    encode_component(value)
}

pub(super) fn encode_query_component(value: &str) -> String {
    encode_component(value)
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                encoded.push('%');
                encoded.push(hex_digit(byte >> 4));
                encoded.push(hex_digit(byte & 0x0f));
            }
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + value - 10),
    }
}

#[cfg(test)]
mod tests {
    use super::{encode_path_segment, encode_query_component};

    #[test]
    fn component_encoding_preserves_only_unreserved_bytes() {
        assert_eq!(
            encode_path_segment("AZaz09-._~:/?#% \u{96ea}"),
            "AZaz09-._~%3A%2F%3F%23%25%20%E9%9B%AA"
        );
        assert_eq!(
            encode_query_component("opp/id?cursor=1&next=2"),
            "opp%2Fid%3Fcursor%3D1%26next%3D2"
        );
    }
}

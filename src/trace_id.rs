/// Generate a 128-bit trace ID.
///
/// If a CloudFront request ID is provided, derive deterministically via BLAKE3.
/// Otherwise, generate a random UUID v4.
pub fn generate_trace_id(cf_id: Option<&str>) -> [u8; 16] {
    match cf_id {
        Some(id) if !id.is_empty() && id != "-" => {
            let hash = blake3::hash(id.as_bytes());
            let mut trace_id = [0u8; 16];
            trace_id.copy_from_slice(&hash.as_bytes()[..16]);
            trace_id
        }
        _ => *uuid::Uuid::new_v4().as_bytes(),
    }
}

/// Generate a random 64-bit span ID.
pub fn generate_span_id() -> [u8; 8] {
    let mut span_id = [0u8; 8];
    rand::fill(&mut span_id);
    span_id
}

/// Encode bytes as lowercase hex. Used for trace_id/span_id display.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_cf_id() {
        let id1 = generate_trace_id(Some("abc123"));
        let id2 = generate_trace_id(Some("abc123"));
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_cf_ids_produce_different_trace_ids() {
        let id1 = generate_trace_id(Some("abc123"));
        let id2 = generate_trace_id(Some("xyz789"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn none_produces_nonzero() {
        let id = generate_trace_id(None);
        assert_ne!(id, [0u8; 16]);
    }

    #[test]
    fn dash_treated_as_none() {
        let id1 = generate_trace_id(Some("-"));
        let id2 = generate_trace_id(Some("-"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn empty_string_treated_as_none() {
        let id1 = generate_trace_id(Some(""));
        let id2 = generate_trace_id(Some(""));
        assert_ne!(id1, id2);
    }

    #[test]
    fn span_id_is_nonzero() {
        let id = generate_span_id();
        assert_ne!(id, [0u8; 8]);
    }

    #[test]
    fn span_ids_are_unique() {
        let id1 = generate_span_id();
        let id2 = generate_span_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn hex_encode_works() {
        assert_eq!(hex_encode(&[0x0A, 0xFF, 0x00]), "0aff00");
        assert_eq!(hex_encode(&[]), "");
    }
}

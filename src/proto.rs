/// Wire types per protobuf spec.
const WIRE_TYPE_VARINT: u8 = 0;
const WIRE_TYPE_FIXED64: u8 = 1;
const WIRE_TYPE_LENGTH_DELIMITED: u8 = 2;

/// Encode a varint (LEB128) into the buffer.
fn encode_varint(buf: &mut Vec<u8>, mut val: u64) {
    while val >= 0x80 {
        buf.push((val as u8) | 0x80);
        val >>= 7;
    }
    buf.push(val as u8);
}

/// Encode a field tag (field number + wire type).
fn encode_tag(buf: &mut Vec<u8>, field_number: u32, wire_type: u8) {
    encode_varint(buf, ((field_number as u64) << 3) | wire_type as u64);
}

/// Encode a varint field (tag + varint value).
/// Follows proto3 convention: skips the field if val == 0.
pub(crate) fn encode_varint_field(buf: &mut Vec<u8>, field: u32, val: u64) {
    if val == 0 {
        return;
    }
    encode_tag(buf, field, WIRE_TYPE_VARINT);
    encode_varint(buf, val);
}

/// Encode a varint field unconditionally, even if val == 0.
/// Use for fields where zero is a meaningful value (e.g. bool false, int 0).
pub(crate) fn encode_varint_field_always(buf: &mut Vec<u8>, field: u32, val: u64) {
    encode_tag(buf, field, WIRE_TYPE_VARINT);
    encode_varint(buf, val);
}

/// Encode a string field (tag + length + UTF-8 bytes).
/// Skips the field if the string is empty (proto3 default).
pub(crate) fn encode_string_field(buf: &mut Vec<u8>, field: u32, s: &str) {
    if s.is_empty() {
        return;
    }
    encode_bytes_field(buf, field, s.as_bytes());
}

/// Encode a bytes field (tag + length + raw bytes).
/// Skips the field if the slice is empty (proto3 default).
pub(crate) fn encode_bytes_field(buf: &mut Vec<u8>, field: u32, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    encode_tag(buf, field, WIRE_TYPE_LENGTH_DELIMITED);
    encode_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

/// Encode a fixed64 field (tag + 8 bytes little-endian).
/// Skips the field if val == 0 (proto3 default).
pub(crate) fn encode_fixed64_field(buf: &mut Vec<u8>, field: u32, val: u64) {
    if val == 0 {
        return;
    }
    encode_tag(buf, field, WIRE_TYPE_FIXED64);
    buf.extend_from_slice(&val.to_le_bytes());
}

/// Encode a fixed64 field unconditionally, even if val == 0.
pub(crate) fn encode_fixed64_field_always(buf: &mut Vec<u8>, field: u32, val: u64) {
    encode_tag(buf, field, WIRE_TYPE_FIXED64);
    buf.extend_from_slice(&val.to_le_bytes());
}

/// Encode a nested message field (tag + length + message bytes).
/// Skips the field if the message is empty.
pub(crate) fn encode_message_field(buf: &mut Vec<u8>, field: u32, msg: &[u8]) {
    if msg.is_empty() {
        return;
    }
    encode_tag(buf, field, WIRE_TYPE_LENGTH_DELIMITED);
    encode_varint(buf, msg.len() as u64);
    buf.extend_from_slice(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_zero() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 0);
        assert_eq!(buf, vec![0x00]);
    }

    #[test]
    fn varint_one() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 1);
        assert_eq!(buf, vec![0x01]);
    }

    #[test]
    fn varint_127() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 127);
        assert_eq!(buf, vec![0x7F]);
    }

    #[test]
    fn varint_128() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 128);
        assert_eq!(buf, vec![0x80, 0x01]);
    }

    #[test]
    fn varint_300() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 300);
        assert_eq!(buf, vec![0xAC, 0x02]);
    }

    #[test]
    fn string_field_encoding() {
        let mut buf = Vec::new();
        encode_string_field(&mut buf, 1, "hi");
        assert_eq!(buf, vec![0x0A, 0x02, b'h', b'i']);
    }

    #[test]
    fn string_field_empty_is_skipped() {
        let mut buf = Vec::new();
        encode_string_field(&mut buf, 1, "");
        assert!(buf.is_empty());
    }

    #[test]
    fn fixed64_encoding() {
        let mut buf = Vec::new();
        encode_fixed64_field(&mut buf, 1, 0x0102030405060708);
        assert_eq!(
            buf,
            vec![0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
    }

    #[test]
    fn fixed64_zero_is_skipped() {
        let mut buf = Vec::new();
        encode_fixed64_field(&mut buf, 1, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn fixed64_always_encodes_zero() {
        let mut buf = Vec::new();
        encode_fixed64_field_always(&mut buf, 1, 0);
        assert_eq!(buf, vec![0x09, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn varint_always_encodes_zero() {
        let mut buf = Vec::new();
        encode_varint_field_always(&mut buf, 2, 0);
        // tag = (2<<3)|0 = 0x10, value = 0x00
        assert_eq!(buf, vec![0x10, 0x00]);
    }

    #[test]
    fn nested_message_encoding() {
        let mut inner = Vec::new();
        encode_string_field(&mut inner, 1, "ab");

        let mut buf = Vec::new();
        encode_message_field(&mut buf, 2, &inner);

        assert_eq!(buf, vec![0x12, 0x04, 0x0A, 0x02, b'a', b'b']);
    }

    #[test]
    fn varint_field_encoding() {
        let mut buf = Vec::new();
        encode_varint_field(&mut buf, 3, 150);
        assert_eq!(buf, vec![0x18, 0x96, 0x01]);
    }

    #[test]
    fn varint_field_zero_is_skipped() {
        let mut buf = Vec::new();
        encode_varint_field(&mut buf, 3, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn bytes_field_encoding() {
        let mut buf = Vec::new();
        encode_bytes_field(&mut buf, 1, &[0xDE, 0xAD]);
        assert_eq!(buf, vec![0x0A, 0x02, 0xDE, 0xAD]);
    }
}

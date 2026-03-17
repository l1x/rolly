#![cfg(feature = "_bench")]

//! Property-based tests for protobuf encoding primitives using proptest.
//! Verifies structural correctness of our hand-rolled encoding.

use proptest::prelude::*;
use rolly::bench::{
    encode_bytes_field,
    // Higher-level encoding
    encode_export_logs_request,
    encode_export_trace_request,
    encode_message_field,
    encode_message_field_in_place,
    encode_string_field,
    encode_varint_field,
    hex_encode,
    hex_to_bytes_16,
    AnyValue,
    KeyValue,
    LogData,
    SeverityNumber,
    SpanData,
    SpanKind,
    SpanStatus,
    StatusCode,
};

/// Decode a varint from a byte slice, returning (value, bytes_consumed).
fn decode_varint(buf: &[u8]) -> (u64, usize) {
    let mut val: u64 = 0;
    let mut shift = 0;
    for (i, &b) in buf.iter().enumerate() {
        val |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return (val, i + 1);
        }
        shift += 7;
    }
    panic!("unterminated varint");
}

proptest! {
    #[test]
    fn varint_field_has_valid_tag_and_value(val in 1u64..=u64::MAX) {
        // val > 0 because encode_varint_field skips zero
        let field_num = 1u32;
        let mut buf = Vec::new();
        encode_varint_field(&mut buf, field_num, val);

        // Decode the tag
        let (tag, tag_len) = decode_varint(&buf);
        let wire_type = tag & 0x07;
        let decoded_field = tag >> 3;
        prop_assert_eq!(wire_type, 0, "wire type should be VARINT (0)");
        prop_assert_eq!(decoded_field, field_num as u64);

        // Decode the value
        let (decoded_val, val_len) = decode_varint(&buf[tag_len..]);
        prop_assert_eq!(decoded_val, val);

        // Consumed all bytes
        prop_assert_eq!(tag_len + val_len, buf.len());
    }

    #[test]
    fn varint_field_zero_is_skipped(field_num in 1u32..100) {
        let mut buf = Vec::new();
        encode_varint_field(&mut buf, field_num, 0);
        prop_assert!(buf.is_empty());
    }

    #[test]
    fn string_field_roundtrip(s in "\\PC{0,200}") {
        if s.is_empty() {
            // empty strings are skipped by proto3 convention
            let mut buf = Vec::new();
            encode_string_field(&mut buf, 1, &s);
            prop_assert!(buf.is_empty());
        } else {
            let mut buf = Vec::new();
            encode_string_field(&mut buf, 1, &s);

            // Decode tag
            let (tag, tag_len) = decode_varint(&buf);
            let wire_type = tag & 0x07;
            prop_assert_eq!(wire_type, 2, "wire type should be LENGTH_DELIMITED (2)");

            // Decode length prefix
            let (length, len_len) = decode_varint(&buf[tag_len..]);
            prop_assert_eq!(length as usize, s.len());

            // Verify body matches
            let body_start = tag_len + len_len;
            let body = &buf[body_start..];
            prop_assert_eq!(body, s.as_bytes());
        }
    }

    #[test]
    fn bytes_field_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..300)) {
        let mut buf = Vec::new();
        encode_bytes_field(&mut buf, 1, &data);

        if data.is_empty() {
            prop_assert!(buf.is_empty());
        } else {
            // Decode tag
            let (tag, tag_len) = decode_varint(&buf);
            let wire_type = tag & 0x07;
            prop_assert_eq!(wire_type, 2);

            // Decode length prefix
            let (length, len_len) = decode_varint(&buf[tag_len..]);
            prop_assert_eq!(length as usize, data.len());

            // Verify body
            let body_start = tag_len + len_len;
            prop_assert_eq!(&buf[body_start..], &data[..]);
        }
    }

    #[test]
    fn message_field_in_place_matches_allocating(
        body in proptest::collection::vec(any::<u8>(), 0..500),
        field_num in 1u32..50
    ) {
        // Allocating approach
        let mut expected = Vec::new();
        encode_message_field(&mut expected, field_num, &body);

        // In-place approach
        let mut actual = Vec::new();
        encode_message_field_in_place(&mut actual, field_num, |buf| {
            buf.extend_from_slice(&body);
        });

        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn string_field_various_field_numbers(
        field_num in 1u32..1000,
        s in "[a-z]{1,20}"
    ) {
        let mut buf = Vec::new();
        encode_string_field(&mut buf, field_num, &s);

        let (tag, tag_len) = decode_varint(&buf);
        let decoded_field = tag >> 3;
        let wire_type = tag & 0x07;
        prop_assert_eq!(decoded_field, field_num as u64);
        prop_assert_eq!(wire_type, 2);

        let (length, len_len) = decode_varint(&buf[tag_len..]);
        prop_assert_eq!(length as usize, s.len());
        prop_assert_eq!(tag_len + len_len + s.len(), buf.len());
    }
}

// --- Strategy helpers for higher-level encoding proptests ---

fn arb_any_value() -> impl Strategy<Value = AnyValue> {
    prop_oneof![
        "[a-zA-Z0-9 ]{0,64}".prop_map(AnyValue::String),
        any::<i64>().prop_map(AnyValue::Int),
        any::<bool>().prop_map(AnyValue::Bool),
        any::<f64>().prop_map(AnyValue::Double),
        proptest::collection::vec(any::<u8>(), 0..32).prop_map(AnyValue::Bytes),
    ]
}

fn arb_key_value() -> impl Strategy<Value = KeyValue> {
    ("[a-zA-Z][a-zA-Z0-9_.]{0,31}", arb_any_value())
        .prop_map(|(key, value)| KeyValue { key, value })
}

fn arb_span_kind() -> impl Strategy<Value = SpanKind> {
    prop_oneof![
        Just(SpanKind::Unspecified),
        Just(SpanKind::Internal),
        Just(SpanKind::Server),
        Just(SpanKind::Client),
        Just(SpanKind::Producer),
        Just(SpanKind::Consumer),
    ]
}

fn arb_status_code() -> impl Strategy<Value = StatusCode> {
    prop_oneof![
        Just(StatusCode::Unset),
        Just(StatusCode::Ok),
        Just(StatusCode::Error),
    ]
}

fn arb_span_data() -> impl Strategy<Value = SpanData> {
    (
        any::<[u8; 16]>(),              // trace_id
        any::<[u8; 8]>(),               // span_id
        any::<[u8; 8]>(),               // parent_span_id
        "[a-zA-Z][a-zA-Z0-9_. ]{0,63}", // name
        arb_span_kind(),
        any::<u64>(),                                     // start_time
        any::<u64>(),                                     // end_time
        proptest::collection::vec(arb_key_value(), 0..8), // attributes
        proptest::option::of(
            ("[a-zA-Z0-9 ]{0,32}", arb_status_code())
                .prop_map(|(msg, code)| SpanStatus { message: msg, code }),
        ),
    )
        .prop_map(
            |(trace_id, span_id, parent_span_id, name, kind, start, end, attributes, status)| {
                SpanData {
                    trace_id,
                    span_id,
                    parent_span_id,
                    name,
                    kind,
                    start_time_unix_nano: start,
                    end_time_unix_nano: end,
                    attributes,
                    status,
                }
            },
        )
}

fn arb_severity_number() -> impl Strategy<Value = SeverityNumber> {
    prop_oneof![
        Just(SeverityNumber::Trace),
        Just(SeverityNumber::Debug),
        Just(SeverityNumber::Info),
        Just(SeverityNumber::Warn),
        Just(SeverityNumber::Error),
        Just(SeverityNumber::Fatal),
    ]
}

fn arb_log_data() -> impl Strategy<Value = LogData> {
    (
        any::<u64>(), // time_unix_nano
        arb_severity_number(),
        "[A-Z]{1,8}",                                     // severity_text
        arb_any_value(),                                  // body
        proptest::collection::vec(arb_key_value(), 0..8), // attributes
        any::<[u8; 16]>(),                                // trace_id
        any::<[u8; 8]>(),                                 // span_id
    )
        .prop_map(
            |(
                time_unix_nano,
                severity_number,
                severity_text,
                body,
                attributes,
                trace_id,
                span_id,
            )| {
                LogData {
                    time_unix_nano,
                    severity_number,
                    severity_text,
                    body,
                    attributes,
                    trace_id,
                    span_id,
                }
            },
        )
}

proptest! {
    #[test]
    fn encode_trace_request_no_panic(
        spans in proptest::collection::vec(arb_span_data(), 1..=4),
        resource_attrs in proptest::collection::vec(arb_key_value(), 0..4),
    ) {
        let buf = encode_export_trace_request(&resource_attrs, "rolly", "0.1.0", &spans);
        prop_assert!(!buf.is_empty());
    }

    #[test]
    fn encode_logs_request_no_panic(
        logs in proptest::collection::vec(arb_log_data(), 1..=4),
        resource_attrs in proptest::collection::vec(arb_key_value(), 0..4),
    ) {
        let buf = encode_export_logs_request(&resource_attrs, "rolly", "0.1.0", &logs);
        prop_assert!(!buf.is_empty());
    }

    #[test]
    fn hex_to_bytes_16_roundtrip(s in "\\PC{0,64}") {
        let result = hex_to_bytes_16(&s);
        let is_valid_hex_32 = s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit());
        if is_valid_hex_32 {
            let bytes = result.unwrap();
            let roundtrip = hex_encode(&bytes);
            prop_assert_eq!(roundtrip, s.to_ascii_lowercase());
        } else {
            prop_assert!(result.is_err());
        }
    }
}

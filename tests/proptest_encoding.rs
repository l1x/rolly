#![cfg(feature = "_bench")]

//! Property-based tests for protobuf encoding primitives using proptest.
//! Verifies structural correctness of our hand-rolled encoding.

use proptest::prelude::*;
use rolly::bench::{
    encode_bytes_field, encode_message_field, encode_message_field_in_place, encode_string_field,
    encode_varint_field,
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

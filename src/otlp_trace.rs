use crate::proto::*;

// --- Data types ---

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AnyValue {
    String(String),
    Int(i64),
    Bool(bool),
    Double(f64),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[allow(dead_code)]
pub enum SpanKind {
    Unspecified = 0,
    Internal = 1,
    Server = 2,
    Client = 3,
    Producer = 4,
    Consumer = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[allow(dead_code)]
pub enum StatusCode {
    Unset = 0,
    Ok = 1,
    Error = 2,
}

#[derive(Debug, Clone)]
pub struct SpanStatus {
    pub message: String,
    pub code: StatusCode,
}

#[derive(Debug, Clone)]
pub struct SpanData {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent_span_id: [u8; 8],
    pub name: String,
    pub kind: SpanKind,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub attributes: Vec<KeyValue>,
    pub status: Option<SpanStatus>,
}

// --- Encoding ---

/// Encode an AnyValue into a protobuf AnyValue message.
/// AnyValue proto: oneof value { string_value=1, bool_value=2, int_value=3, double_value=4, bytes_value=7 }
///
/// Uses `_always` variants because zero/false/0.0 are valid attribute values
/// that must be encoded (they're inside a oneof, not default scalars).
pub(crate) fn encode_any_value(buf: &mut Vec<u8>, val: &AnyValue) {
    match val {
        AnyValue::String(s) => encode_string_field(buf, 1, s),
        AnyValue::Bool(b) => encode_varint_field_always(buf, 2, *b as u64),
        AnyValue::Int(i) => encode_varint_field_always(buf, 3, *i as u64),
        AnyValue::Double(d) => encode_fixed64_field_always(buf, 4, d.to_bits()),
        AnyValue::Bytes(b) => encode_bytes_field(buf, 7, b),
    }
}

/// Encode a KeyValue: field 1 = key (string), field 2 = AnyValue (message).
pub(crate) fn encode_key_value(buf: &mut Vec<u8>, kv: &KeyValue) {
    encode_string_field(buf, 1, &kv.key);
    let mut val_buf = Vec::new();
    encode_any_value(&mut val_buf, &kv.value);
    encode_message_field(buf, 2, &val_buf);
}

/// Encode a Resource: field 1 = repeated KeyValue (attributes).
pub(crate) fn encode_resource(buf: &mut Vec<u8>, attrs: &[KeyValue]) {
    for kv in attrs {
        let mut kv_buf = Vec::new();
        encode_key_value(&mut kv_buf, kv);
        encode_message_field(buf, 1, &kv_buf);
    }
}

/// Encode an InstrumentationScope: field 1 = name, field 2 = version.
pub(crate) fn encode_scope(buf: &mut Vec<u8>, name: &str, version: &str) {
    encode_string_field(buf, 1, name);
    encode_string_field(buf, 2, version);
}

/// Encode a Status message: field 2 = message, field 3 = code.
fn encode_status(buf: &mut Vec<u8>, status: &SpanStatus) {
    encode_string_field(buf, 2, &status.message);
    encode_varint_field(buf, 3, status.code as u64);
}

/// Encode a Span message per OTLP proto field numbers:
/// trace_id(1), span_id(2), parent_span_id(4), name(5), kind(6),
/// start_time_unix_nano(7 fixed64), end_time_unix_nano(8 fixed64),
/// attributes(9 repeated), status(15)
fn encode_span(buf: &mut Vec<u8>, span: &SpanData) {
    encode_bytes_field(buf, 1, &span.trace_id);
    encode_bytes_field(buf, 2, &span.span_id);
    // field 3 = trace_state (skipped)
    encode_bytes_field(buf, 4, &span.parent_span_id);
    encode_string_field(buf, 5, &span.name);
    encode_varint_field(buf, 6, span.kind as u64);
    encode_fixed64_field(buf, 7, span.start_time_unix_nano);
    encode_fixed64_field(buf, 8, span.end_time_unix_nano);

    for kv in &span.attributes {
        let mut kv_buf = Vec::new();
        encode_key_value(&mut kv_buf, kv);
        encode_message_field(buf, 9, &kv_buf);
    }

    if let Some(ref status) = span.status {
        let mut status_buf = Vec::new();
        encode_status(&mut status_buf, status);
        encode_message_field(buf, 15, &status_buf);
    }
}

/// Encode a full ExportTraceServiceRequest.
///
/// Structure:
///   ExportTraceServiceRequest { resource_spans: \[ResourceSpans\] }
///     ResourceSpans { resource(1), scope_spans(2) }
///       ScopeSpans { scope(1), spans(2) }
pub(crate) fn encode_export_trace_request(
    resource_attrs: &[KeyValue],
    scope_name: &str,
    scope_version: &str,
    spans: &[SpanData],
) -> Vec<u8> {
    let mut spans_buf = Vec::new();
    for span in spans {
        let mut span_buf = Vec::new();
        encode_span(&mut span_buf, span);
        encode_message_field(&mut spans_buf, 2, &span_buf);
    }

    let mut scope_buf = Vec::new();
    encode_scope(&mut scope_buf, scope_name, scope_version);

    let mut scope_spans_buf = Vec::new();
    encode_message_field(&mut scope_spans_buf, 1, &scope_buf);
    scope_spans_buf.extend_from_slice(&spans_buf);

    let mut resource_buf = Vec::new();
    encode_resource(&mut resource_buf, resource_attrs);

    let mut resource_spans_buf = Vec::new();
    encode_message_field(&mut resource_spans_buf, 1, &resource_buf);
    encode_message_field(&mut resource_spans_buf, 2, &scope_spans_buf);

    let mut request_buf = Vec::new();
    encode_message_field(&mut request_buf, 1, &resource_spans_buf);

    request_buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_span() -> SpanData {
        SpanData {
            trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: [1, 2, 3, 4, 5, 6, 7, 8],
            parent_span_id: [0; 8],
            name: "test-span".to_string(),
            kind: SpanKind::Server,
            start_time_unix_nano: 1_000_000_000,
            end_time_unix_nano: 2_000_000_000,
            attributes: vec![KeyValue {
                key: "http.method".to_string(),
                value: AnyValue::String("GET".to_string()),
            }],
            status: Some(SpanStatus {
                message: String::new(),
                code: StatusCode::Ok,
            }),
        }
    }

    #[test]
    fn encode_span_contains_trace_id() {
        let span = test_span();
        let mut buf = Vec::new();
        encode_span(&mut buf, &span);

        assert_eq!(buf[0], 0x0A); // field 1, wire type 2
        assert_eq!(buf[1], 16);
        assert_eq!(&buf[2..18], &span.trace_id);
    }

    #[test]
    fn encode_span_contains_span_id() {
        let span = test_span();
        let mut buf = Vec::new();
        encode_span(&mut buf, &span);

        assert_eq!(buf[18], 0x12); // field 2, wire type 2
        assert_eq!(buf[19], 8);
        assert_eq!(&buf[20..28], &span.span_id);
    }

    #[test]
    fn encode_span_contains_name() {
        let span = test_span();
        let mut buf = Vec::new();
        encode_span(&mut buf, &span);

        let name_bytes = b"test-span";
        let found = buf.windows(name_bytes.len()).any(|w| w == name_bytes);
        assert!(found, "span name not found in encoded bytes");
    }

    #[test]
    fn encode_export_trace_request_is_nonempty() {
        let attrs = vec![KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String("test-svc".to_string()),
        }];
        let spans = vec![test_span()];

        let bytes = encode_export_trace_request(&attrs, "pz-o11y", "0.1.0", &spans);
        assert!(!bytes.is_empty());
        assert_eq!(bytes[0], 0x0A);
    }

    #[test]
    fn encode_key_value_string() {
        let kv = KeyValue {
            key: "k".to_string(),
            value: AnyValue::String("v".to_string()),
        };
        let mut buf = Vec::new();
        encode_key_value(&mut buf, &kv);

        assert_eq!(&buf[0..3], &[0x0A, 0x01, b'k']);
        assert_eq!(&buf[3..], &[0x12, 0x03, 0x0A, 0x01, b'v']);
    }

    #[test]
    fn encode_any_value_bool_true() {
        let mut buf = Vec::new();
        encode_any_value(&mut buf, &AnyValue::Bool(true));
        assert_eq!(buf, vec![0x10, 0x01]);
    }

    #[test]
    fn encode_any_value_bool_false_is_preserved() {
        let mut buf = Vec::new();
        encode_any_value(&mut buf, &AnyValue::Bool(false));
        // Must encode: tag=0x10, value=0x00
        assert_eq!(buf, vec![0x10, 0x00]);
    }

    #[test]
    fn encode_any_value_int_zero_is_preserved() {
        let mut buf = Vec::new();
        encode_any_value(&mut buf, &AnyValue::Int(0));
        // Must encode: tag=0x18, value=0x00
        assert_eq!(buf, vec![0x18, 0x00]);
    }

    #[test]
    fn encode_any_value_double_zero_is_preserved() {
        let mut buf = Vec::new();
        encode_any_value(&mut buf, &AnyValue::Double(0.0));
        // Must encode: tag=0x21 (field 4, wire type 1), then 8 zero bytes
        assert_eq!(buf, vec![0x21, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn encode_any_value_int() {
        let mut buf = Vec::new();
        encode_any_value(&mut buf, &AnyValue::Int(42));
        assert_eq!(buf, vec![0x18, 0x2A]);
    }

    #[test]
    fn encode_status_ok() {
        let status = SpanStatus {
            message: String::new(),
            code: StatusCode::Ok,
        };
        let mut buf = Vec::new();
        encode_status(&mut buf, &status);
        assert_eq!(buf, vec![0x18, 0x01]);
    }
}

#![no_main]

use libfuzzer_sys::fuzz_target;
use rolly::bench::{
    encode_export_trace_request, AnyValue, KeyValue, SpanData, SpanKind, SpanStatus, StatusCode,
};

fn span_kind_from_u8(v: u8) -> SpanKind {
    match v % 6 {
        0 => SpanKind::Unspecified,
        1 => SpanKind::Internal,
        2 => SpanKind::Server,
        3 => SpanKind::Client,
        4 => SpanKind::Producer,
        _ => SpanKind::Consumer,
    }
}

fn status_code_from_u8(v: u8) -> StatusCode {
    match v % 3 {
        0 => StatusCode::Unset,
        1 => StatusCode::Ok,
        _ => StatusCode::Error,
    }
}

fuzz_target!(|data: &[u8]| {
    // Need at least enough bytes for one minimal span
    if data.len() < 40 {
        return;
    }

    let mut pos = 0;

    // Parse a trace_id
    let mut trace_id = [0u8; 16];
    trace_id.copy_from_slice(&data[pos..pos + 16]);
    pos += 16;

    // Parse a span_id
    let mut span_id = [0u8; 8];
    span_id.copy_from_slice(&data[pos..pos + 8]);
    pos += 8;

    // Parse parent_span_id
    let mut parent_span_id = [0u8; 8];
    parent_span_id.copy_from_slice(&data[pos..pos + 8]);
    pos += 8;

    // Parse kind
    let kind = span_kind_from_u8(data[pos]);
    pos += 1;

    // Parse timestamps
    if pos + 16 > data.len() {
        return;
    }
    let start = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let end = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
    pos += 8;

    // Use remaining bytes as span name
    let name_len = if pos < data.len() {
        (data.len() - pos).min(64)
    } else {
        0
    };
    let name = String::from_utf8_lossy(&data[pos..pos + name_len]).to_string();

    // Build status from kind byte
    let status = if data[pos.min(data.len() - 1)] % 3 == 0 {
        None
    } else {
        Some(SpanStatus {
            message: String::new(),
            code: status_code_from_u8(data[pos.min(data.len() - 1)]),
        })
    };

    let span = SpanData {
        trace_id,
        span_id,
        parent_span_id,
        name,
        kind,
        start_time_unix_nano: start,
        end_time_unix_nano: end,
        attributes: vec![KeyValue {
            key: "fuzz".to_string(),
            value: AnyValue::Bool(true),
        }],
        status,
    };

    let resource_attrs = vec![KeyValue {
        key: "service.name".to_string(),
        value: AnyValue::String("fuzz-svc".to_string()),
    }];

    let _ = encode_export_trace_request(&resource_attrs, "rolly", "0.0.0", &[span]);
});

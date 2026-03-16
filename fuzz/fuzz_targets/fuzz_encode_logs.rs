#![no_main]

use libfuzzer_sys::fuzz_target;
use rolly::bench::{
    encode_export_logs_request, AnyValue, KeyValue, LogData, SeverityNumber,
};

fn severity_from_u8(v: u8) -> SeverityNumber {
    match v % 6 {
        0 => SeverityNumber::Trace,
        1 => SeverityNumber::Debug,
        2 => SeverityNumber::Info,
        3 => SeverityNumber::Warn,
        4 => SeverityNumber::Error,
        _ => SeverityNumber::Fatal,
    }
}

fuzz_target!(|data: &[u8]| {
    // Need at least enough bytes for one minimal log record
    if data.len() < 34 {
        return;
    }

    let mut pos = 0;

    // Parse time_unix_nano
    let time = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
    pos += 8;

    // Parse severity
    let severity_number = severity_from_u8(data[pos]);
    pos += 1;

    // Parse trace_id
    let mut trace_id = [0u8; 16];
    trace_id.copy_from_slice(&data[pos..pos + 16]);
    pos += 16;

    // Parse span_id
    let mut span_id = [0u8; 8];
    span_id.copy_from_slice(&data[pos..pos + 8]);
    pos += 8;

    // Parse severity_text byte
    let severity_text_byte = data[pos];
    pos += 1;
    let severity_text = match severity_text_byte % 6 {
        0 => "TRACE",
        1 => "DEBUG",
        2 => "INFO",
        3 => "WARN",
        4 => "ERROR",
        _ => "FATAL",
    }
    .to_string();

    // Use remaining bytes as body
    let body_str = String::from_utf8_lossy(&data[pos..]).to_string();

    let log = LogData {
        time_unix_nano: time,
        severity_number,
        severity_text,
        body: AnyValue::String(body_str),
        attributes: vec![KeyValue {
            key: "fuzz".to_string(),
            value: AnyValue::Bool(true),
        }],
        trace_id,
        span_id,
    };

    let resource_attrs = vec![KeyValue {
        key: "service.name".to_string(),
        value: AnyValue::String("fuzz-svc".to_string()),
    }];

    let _ = encode_export_logs_request(&resource_attrs, "rolly", "0.0.0", &[log]);
});

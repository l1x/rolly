use crate::otlp_trace::{encode_any_value, encode_key_value, encode_resource, encode_scope, AnyValue, KeyValue};
use crate::proto::*;

// --- Data types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
#[allow(dead_code)]
pub enum SeverityNumber {
    Trace = 1,
    Debug = 5,
    Info = 9,
    Warn = 13,
    Error = 17,
    Fatal = 21,
}

#[derive(Debug, Clone)]
pub struct LogData {
    pub time_unix_nano: u64,
    pub severity_number: SeverityNumber,
    pub severity_text: String,
    pub body: AnyValue,
    pub attributes: Vec<KeyValue>,
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
}

// --- Encoding ---

/// Encode a LogRecord per OTLP proto field numbers:
/// time_unix_nano(1 fixed64), severity_number(2), severity_text(3),
/// body(5 AnyValue message), attributes(6 repeated), trace_id(9), span_id(10)
fn encode_log_record(buf: &mut Vec<u8>, log: &LogData) {
    encode_fixed64_field(buf, 1, log.time_unix_nano);
    encode_varint_field(buf, 2, log.severity_number as u64);
    encode_string_field(buf, 3, &log.severity_text);

    let mut body_buf = Vec::new();
    encode_any_value(&mut body_buf, &log.body);
    encode_message_field(buf, 5, &body_buf);

    for kv in &log.attributes {
        let mut kv_buf = Vec::new();
        encode_key_value(&mut kv_buf, kv);
        encode_message_field(buf, 6, &kv_buf);
    }

    encode_bytes_field(buf, 9, &log.trace_id);
    encode_bytes_field(buf, 10, &log.span_id);
}

/// Encode a full ExportLogsServiceRequest.
///
/// Structure:
///   ExportLogsServiceRequest { resource_logs: \[ResourceLogs\] }
///     ResourceLogs { resource(1), scope_logs(2) }
///       ScopeLogs { scope(1), log_records(2) }
pub(crate) fn encode_export_logs_request(
    resource_attrs: &[KeyValue],
    scope_name: &str,
    scope_version: &str,
    logs: &[LogData],
) -> Vec<u8> {
    let mut logs_buf = Vec::new();
    for log in logs {
        let mut log_buf = Vec::new();
        encode_log_record(&mut log_buf, log);
        encode_message_field(&mut logs_buf, 2, &log_buf);
    }

    let mut scope_buf = Vec::new();
    encode_scope(&mut scope_buf, scope_name, scope_version);

    let mut scope_logs_buf = Vec::new();
    encode_message_field(&mut scope_logs_buf, 1, &scope_buf);
    scope_logs_buf.extend_from_slice(&logs_buf);

    let mut resource_buf = Vec::new();
    encode_resource(&mut resource_buf, resource_attrs);

    let mut resource_logs_buf = Vec::new();
    encode_message_field(&mut resource_logs_buf, 1, &resource_buf);
    encode_message_field(&mut resource_logs_buf, 2, &scope_logs_buf);

    let mut request_buf = Vec::new();
    encode_message_field(&mut request_buf, 1, &resource_logs_buf);

    request_buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_log() -> LogData {
        LogData {
            time_unix_nano: 1_000_000_000,
            severity_number: SeverityNumber::Info,
            severity_text: "INFO".to_string(),
            body: AnyValue::String("hello world".to_string()),
            attributes: vec![KeyValue {
                key: "service.name".to_string(),
                value: AnyValue::String("test-svc".to_string()),
            }],
            trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            span_id: [1, 2, 3, 4, 5, 6, 7, 8],
        }
    }

    #[test]
    fn encode_log_record_contains_time() {
        let log = test_log();
        let mut buf = Vec::new();
        encode_log_record(&mut buf, &log);

        assert_eq!(buf[0], 0x09);
        let time_bytes = &buf[1..9];
        let time = u64::from_le_bytes(time_bytes.try_into().unwrap());
        assert_eq!(time, 1_000_000_000);
    }

    #[test]
    fn encode_log_record_contains_severity() {
        let log = test_log();
        let mut buf = Vec::new();
        encode_log_record(&mut buf, &log);

        assert_eq!(buf[9], 0x10);
        assert_eq!(buf[10], 0x09);
    }

    #[test]
    fn encode_log_record_contains_body() {
        let log = test_log();
        let mut buf = Vec::new();
        encode_log_record(&mut buf, &log);

        let body_bytes = b"hello world";
        let found = buf.windows(body_bytes.len()).any(|w| w == body_bytes);
        assert!(found, "log body not found in encoded bytes");
    }

    #[test]
    fn encode_log_record_contains_trace_id() {
        let log = test_log();
        let mut buf = Vec::new();
        encode_log_record(&mut buf, &log);

        let trace_id = &log.trace_id;
        let found = buf.windows(trace_id.len()).any(|w| w == trace_id);
        assert!(found, "trace_id not found in encoded bytes");
    }

    #[test]
    fn encode_export_logs_request_is_nonempty() {
        let attrs = vec![KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String("test-svc".to_string()),
        }];
        let logs = vec![test_log()];

        let bytes = encode_export_logs_request(&attrs, "pz-o11y", "0.1.0", &logs);
        assert!(!bytes.is_empty());
        assert_eq!(bytes[0], 0x0A);
    }
}

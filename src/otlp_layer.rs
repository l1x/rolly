use std::time::SystemTime;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::exporter::Exporter;
use crate::otlp_log::{encode_export_logs_request, LogData, SeverityNumber};
use crate::otlp_trace::{
    encode_export_trace_request, AnyValue, KeyValue, SpanData, SpanKind,
};
use crate::trace_id::generate_span_id;

// --- Span extensions ---

struct SpanTiming {
    start_nanos: u64,
}

/// Span context stored in tracing extensions. Public so PropagationLayer can read it.
pub struct SpanFields {
    pub(crate) attrs: Vec<KeyValue>,
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub(crate) parent_span_id: [u8; 8],
}

// --- Shared visitor ---

/// Visitor that collects tracing fields into KeyValue pairs.
/// Used for both span attributes and event fields.
struct FieldCollector {
    attrs: Vec<KeyValue>,
    trace_id: Option<[u8; 16]>,
    message: Option<String>,
}

impl FieldCollector {
    fn new() -> Self {
        Self {
            attrs: Vec::new(),
            trace_id: None,
            message: None,
        }
    }
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let s = format!("{:?}", value);
        if field.name() == "message" {
            self.message = Some(s);
        } else {
            self.attrs.push(KeyValue {
                key: field.name().to_string(),
                value: AnyValue::String(s),
            });
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            if field.name() == "trace_id" {
                if let Ok(bytes) = hex_to_bytes_16(value) {
                    self.trace_id = Some(bytes);
                }
            }
            self.attrs.push(KeyValue {
                key: field.name().to_string(),
                value: AnyValue::String(value.to_string()),
            });
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.attrs.push(KeyValue {
            key: field.name().to_string(),
            value: AnyValue::Int(value),
        });
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.attrs.push(KeyValue {
            key: field.name().to_string(),
            value: AnyValue::Int(value as i64),
        });
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.attrs.push(KeyValue {
            key: field.name().to_string(),
            value: AnyValue::Bool(value),
        });
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.attrs.push(KeyValue {
            key: field.name().to_string(),
            value: AnyValue::Double(value),
        });
    }
}

// --- Helpers ---

fn hex_to_bytes_16(s: &str) -> Result<[u8; 16], ()> {
    if s.len() != 32 {
        return Err(());
    }
    let mut out = [0u8; 16];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0]).ok_or(())?;
        let lo = hex_nibble(chunk[1]).ok_or(())?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn level_to_severity(level: &tracing::Level) -> SeverityNumber {
    match *level {
        tracing::Level::TRACE => SeverityNumber::Trace,
        tracing::Level::DEBUG => SeverityNumber::Debug,
        tracing::Level::INFO => SeverityNumber::Info,
        tracing::Level::WARN => SeverityNumber::Warn,
        tracing::Level::ERROR => SeverityNumber::Error,
    }
}

// --- Layer ---

/// Custom tracing Layer that encodes spans/events as OTLP protobuf and sends to Exporter.
pub(crate) struct OtlpLayer {
    exporter: Exporter,
    resource_attrs: Vec<KeyValue>,
    scope_name: String,
    scope_version: String,
}

impl OtlpLayer {
    pub fn new(
        exporter: Exporter,
        service_name: &str,
        service_version: &str,
        environment: &str,
    ) -> Self {
        let resource_attrs = vec![
            KeyValue {
                key: "service.name".to_string(),
                value: AnyValue::String(service_name.to_string()),
            },
            KeyValue {
                key: "service.version".to_string(),
                value: AnyValue::String(service_version.to_string()),
            },
            KeyValue {
                key: "deployment.environment".to_string(),
                value: AnyValue::String(environment.to_string()),
            },
        ];
        Self {
            exporter,
            resource_attrs,
            scope_name: "pz-o11y".to_string(),
            scope_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl<S> Layer<S> for OtlpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span not found");

        let mut visitor = FieldCollector::new();
        attrs.record(&mut visitor);

        let span_id = generate_span_id();
        let trace_id = visitor.trace_id.unwrap_or([0u8; 16]);

        let parent_span_id = span
            .parent()
            .and_then(|p| p.extensions().get::<SpanFields>().map(|f| f.span_id))
            .unwrap_or([0u8; 8]);

        let mut ext = span.extensions_mut();
        ext.insert(SpanTiming {
            start_nanos: now_nanos(),
        });
        ext.insert(SpanFields {
            attrs: visitor.attrs,
            trace_id,
            span_id,
            parent_span_id,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span not found");
        let mut ext = span.extensions_mut();
        if let Some(fields) = ext.get_mut::<SpanFields>() {
            let mut visitor = FieldCollector::new();
            values.record(&mut visitor);
            fields.attrs.extend(visitor.attrs);
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = FieldCollector::new();
        event.record(&mut visitor);

        let (trace_id, span_id) = ctx
            .current_span()
            .id()
            .and_then(|id| {
                ctx.span(id)
                    .and_then(|s| {
                        s.extensions()
                            .get::<SpanFields>()
                            .map(|f| (f.trace_id, f.span_id))
                    })
            })
            .unwrap_or(([0u8; 16], [0u8; 8]));

        let severity = level_to_severity(event.metadata().level());
        let log = LogData {
            time_unix_nano: now_nanos(),
            severity_number: severity,
            severity_text: event.metadata().level().to_string(),
            body: AnyValue::String(visitor.message.unwrap_or_default()),
            attributes: visitor.attrs,
            trace_id,
            span_id,
        };

        let data = encode_export_logs_request(
            &self.resource_attrs,
            &self.scope_name,
            &self.scope_version,
            &[log],
        );
        self.exporter.send_logs(data);
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("span not found");
        let ext = span.extensions();

        let (start_nanos, attrs, trace_id, span_id, parent_span_id) = {
            let timing = match ext.get::<SpanTiming>() {
                Some(t) => t,
                None => return,
            };
            let fields = match ext.get::<SpanFields>() {
                Some(f) => f,
                None => return,
            };
            (
                timing.start_nanos,
                fields.attrs.clone(),
                fields.trace_id,
                fields.span_id,
                fields.parent_span_id,
            )
        };

        let end_nanos = now_nanos();

        let span_data = SpanData {
            trace_id,
            span_id,
            parent_span_id,
            name: span.name().to_string(),
            kind: SpanKind::Internal,
            start_time_unix_nano: start_nanos,
            end_time_unix_nano: end_nanos,
            attributes: attrs,
            status: None,
        };

        let data = encode_export_trace_request(
            &self.resource_attrs,
            &self.scope_name,
            &self.scope_version,
            &[span_data],
        );
        self.exporter.send_traces(data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exporter::ExporterConfig;

    #[tokio::test]
    async fn otlp_layer_constructs_without_panic() {
        let exporter = Exporter::start(ExporterConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            channel_capacity: 64,
        });
        let _layer = OtlpLayer::new(exporter, "test-svc", "0.0.1", "test");
    }

    #[test]
    fn hex_to_bytes_16_valid() {
        let result = hex_to_bytes_16("0102030405060708090a0b0c0d0e0f10");
        assert_eq!(
            result,
            Ok([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
    }

    #[test]
    fn hex_to_bytes_16_wrong_length() {
        assert!(hex_to_bytes_16("0102").is_err());
    }

    #[test]
    fn hex_to_bytes_16_invalid_chars() {
        assert!(hex_to_bytes_16("zz02030405060708090a0b0c0d0e0f10").is_err());
    }
}

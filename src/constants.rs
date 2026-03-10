/// Span field name constants.
///
/// Use these when creating spans that OtlpLayer should capture.
/// In particular, a span field named `TRACE_ID` with a 32-char hex value
/// will be parsed by OtlpLayer's FieldCollector as the W3C trace ID.
pub mod fields {
    pub const TRACE_ID: &str = "trace_id";
    pub const SPAN_ID: &str = "span_id";
    pub const HTTP_METHOD: &str = "http.method";
    pub const HTTP_URI: &str = "http.uri";
    pub const HTTP_STATUS_CODE: &str = "http.status_code";
    pub const HTTP_LATENCY_MS: &str = "http.latency_ms";
    pub const CF_REQUEST_ID: &str = "cf.request_id";
}

/// Metric event name constants.
///
/// Used in `tracing::info!` events with `metric`, `type`, and `value` fields.
/// These are converted to actual metrics downstream by Vector's `log_to_metric` transform.
pub mod metrics {
    pub const REQUEST_DURATION: &str = "http.server.request.duration";
    pub const REQUEST_COUNT: &str = "http.server.request.count";
    pub const ERROR_COUNT: &str = "http.server.error.count";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn field_constants_are_non_empty() {
        assert!(!fields::TRACE_ID.is_empty());
        assert!(!fields::SPAN_ID.is_empty());
        assert!(!fields::HTTP_METHOD.is_empty());
        assert!(!fields::HTTP_URI.is_empty());
        assert!(!fields::HTTP_STATUS_CODE.is_empty());
        assert!(!fields::HTTP_LATENCY_MS.is_empty());
        assert!(!fields::CF_REQUEST_ID.is_empty());
    }

    #[test]
    #[allow(clippy::const_is_empty)]
    fn metric_constants_are_non_empty() {
        assert!(!metrics::REQUEST_DURATION.is_empty());
        assert!(!metrics::REQUEST_COUNT.is_empty());
        assert!(!metrics::ERROR_COUNT.is_empty());
    }

    #[test]
    fn trace_id_constant_matches_field_collector() {
        // FieldCollector in otlp_layer.rs uses the literal "trace_id"
        assert_eq!(fields::TRACE_ID, "trace_id");
    }
}

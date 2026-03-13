use crate::metrics::{CounterDataPoint, Exemplar, ExemplarValue, GaugeDataPoint, MetricSnapshot};
use crate::otlp_trace::{encode_resource, encode_scope, KeyValue};
use crate::proto::*;

/// Encode a KeyValue from (String, String) attribute pairs.
/// Uses field 1 = key (string), field 2 = value (AnyValue message with string_value field 1).
fn encode_attr_key_value(buf: &mut Vec<u8>, key: &str, value: &str) {
    encode_string_field(buf, 1, key);
    encode_message_field_in_place(buf, 2, |buf| {
        encode_string_field(buf, 1, value); // AnyValue.string_value = field 1
    });
}

/// Encode an Exemplar message.
///
/// Exemplar fields:
///   time_unix_nano(2), as_double(3), span_id(4), trace_id(5), as_int(6)
fn encode_exemplar(buf: &mut Vec<u8>, exemplar: &Exemplar) {
    encode_fixed64_field(buf, 2, exemplar.time_unix_nano);
    match exemplar.value {
        ExemplarValue::Double(v) => {
            encode_fixed64_field_always(buf, 3, v.to_bits());
        }
        ExemplarValue::Int(v) => {
            encode_fixed64_field_always(buf, 6, v as u64);
        }
    }
    encode_bytes_field(buf, 4, &exemplar.span_id);
    encode_bytes_field(buf, 5, &exemplar.trace_id);
}

/// Encode a NumberDataPoint for a counter (as_int, field 6 = sfixed64).
///
/// NumberDataPoint fields:
///   attributes(7), start_time_unix_nano(2), time_unix_nano(3), exemplars(5), as_int(6)
fn encode_counter_data_point(
    buf: &mut Vec<u8>,
    attrs: &[(String, String)],
    start_time_unix_nano: u64,
    time_unix_nano: u64,
    value: i64,
    exemplar: &Option<Exemplar>,
) {
    encode_fixed64_field(buf, 2, start_time_unix_nano);
    encode_fixed64_field(buf, 3, time_unix_nano);
    if let Some(ex) = exemplar {
        encode_message_field_in_place(buf, 5, |buf| {
            encode_exemplar(buf, ex);
        });
    }
    // as_int: field 6, fixed64 (sfixed64 on the wire)
    encode_fixed64_field_always(buf, 6, value as u64);
    for (k, v) in attrs {
        encode_message_field_in_place(buf, 7, |buf| {
            encode_attr_key_value(buf, k, v);
        });
    }
}

/// Encode a NumberDataPoint for a gauge (as_double, field 4 = fixed64).
///
/// NumberDataPoint fields:
///   attributes(7), start_time_unix_nano(2), time_unix_nano(3), as_double(4), exemplars(5)
fn encode_gauge_data_point(
    buf: &mut Vec<u8>,
    attrs: &[(String, String)],
    start_time_unix_nano: u64,
    time_unix_nano: u64,
    value: f64,
    exemplar: &Option<Exemplar>,
) {
    encode_fixed64_field(buf, 2, start_time_unix_nano);
    encode_fixed64_field(buf, 3, time_unix_nano);
    // as_double: field 4, fixed64
    encode_fixed64_field_always(buf, 4, value.to_bits());
    if let Some(ex) = exemplar {
        encode_message_field_in_place(buf, 5, |buf| {
            encode_exemplar(buf, ex);
        });
    }
    for (k, v) in attrs {
        encode_message_field_in_place(buf, 7, |buf| {
            encode_attr_key_value(buf, k, v);
        });
    }
}

/// Encode a Sum message (field 7 of Metric).
///
/// Sum fields:
///   data_points(1), aggregation_temporality(2), is_monotonic(3)
fn encode_sum(buf: &mut Vec<u8>, data_points: &[CounterDataPoint], start_time: u64, time: u64) {
    for (attrs, value, exemplar) in data_points {
        encode_message_field_in_place(buf, 1, |buf| {
            encode_counter_data_point(buf, attrs, start_time, time, *value, exemplar);
        });
    }
    // CUMULATIVE = 2
    encode_varint_field(buf, 2, 2);
    // is_monotonic = true
    encode_varint_field(buf, 3, 1);
}

/// Encode a Gauge message (field 5 of Metric).
///
/// Gauge fields:
///   data_points(1)
fn encode_gauge_msg(buf: &mut Vec<u8>, data_points: &[GaugeDataPoint], start_time: u64, time: u64) {
    for (attrs, value, exemplar) in data_points {
        encode_message_field_in_place(buf, 1, |buf| {
            encode_gauge_data_point(buf, attrs, start_time, time, *value, exemplar);
        });
    }
}

/// Encode a HistogramDataPoint.
///
/// HistogramDataPoint fields:
///   start_time_unix_nano(2), time_unix_nano(3), count(4), sum(5),
///   bucket_counts(6), explicit_bounds(7), attributes(9), min(11), max(12)
fn encode_histogram_data_point(
    buf: &mut Vec<u8>,
    dp: &crate::metrics::HistogramDataPoint,
    boundaries: &[f64],
    start_time_unix_nano: u64,
    time_unix_nano: u64,
) {
    encode_fixed64_field(buf, 2, start_time_unix_nano);
    encode_fixed64_field(buf, 3, time_unix_nano);
    // count: field 4, fixed64 — always encode
    encode_fixed64_field_always(buf, 4, dp.count);
    // sum: field 5, double as fixed64 bits — always encode
    encode_fixed64_field_always(buf, 5, dp.sum.to_bits());
    // bucket_counts: field 6, packed repeated fixed64
    encode_packed_fixed64_field(buf, 6, &dp.bucket_counts);
    // explicit_bounds: field 7, packed repeated double
    encode_packed_double_field(buf, 7, boundaries);
    // attributes: field 9, repeated KeyValue
    for (k, v) in dp.attrs.iter() {
        encode_message_field_in_place(buf, 9, |buf| {
            encode_attr_key_value(buf, k, v);
        });
    }
    // exemplars: field 8, repeated Exemplar
    if let Some(ref ex) = dp.exemplar {
        encode_message_field_in_place(buf, 8, |buf| {
            encode_exemplar(buf, ex);
        });
    }
    // min: field 11, double as fixed64 bits — always encode
    encode_fixed64_field_always(buf, 11, dp.min.to_bits());
    // max: field 12, double as fixed64 bits — always encode
    encode_fixed64_field_always(buf, 12, dp.max.to_bits());
}

/// Encode a Histogram message (field 9 of Metric).
///
/// Histogram fields:
///   data_points(1), aggregation_temporality(2)
fn encode_histogram_msg(
    buf: &mut Vec<u8>,
    data_points: &[crate::metrics::HistogramDataPoint],
    boundaries: &[f64],
    start_time: u64,
    time: u64,
) {
    for dp in data_points {
        encode_message_field_in_place(buf, 1, |buf| {
            encode_histogram_data_point(buf, dp, boundaries, start_time, time);
        });
    }
    // CUMULATIVE = 2
    encode_varint_field(buf, 2, 2);
}

/// Encode a single Metric message.
///
/// Metric fields:
///   name(1), description(2), unit(3), gauge(5), sum(7), histogram(9)
fn encode_metric(buf: &mut Vec<u8>, snapshot: &MetricSnapshot, start_time: u64, time: u64) {
    match snapshot {
        MetricSnapshot::Counter {
            name,
            description,
            data_points,
        } => {
            encode_string_field(buf, 1, name);
            encode_string_field(buf, 2, description);
            // Sum (field 7)
            encode_message_field_in_place(buf, 7, |buf| {
                encode_sum(buf, data_points, start_time, time);
            });
        }
        MetricSnapshot::Gauge {
            name,
            description,
            data_points,
        } => {
            encode_string_field(buf, 1, name);
            encode_string_field(buf, 2, description);
            // Gauge (field 5)
            encode_message_field_in_place(buf, 5, |buf| {
                encode_gauge_msg(buf, data_points, start_time, time);
            });
        }
        MetricSnapshot::Histogram {
            name,
            description,
            boundaries,
            data_points,
        } => {
            encode_string_field(buf, 1, name);
            encode_string_field(buf, 2, description);
            // Histogram (field 9)
            encode_message_field_in_place(buf, 9, |buf| {
                encode_histogram_msg(buf, data_points, boundaries, start_time, time);
            });
        }
    }
}

/// Encode a full ExportMetricsServiceRequest.
///
/// Structure:
///   ExportMetricsServiceRequest { resource_metrics: [ResourceMetrics] }
///     ResourceMetrics { resource(1), scope_metrics(2) }
///       ScopeMetrics { scope(1), metrics(2) }
pub fn encode_export_metrics_request(
    resource_attrs: &[KeyValue],
    scope_name: &str,
    scope_version: &str,
    snapshots: &[MetricSnapshot],
    start_time_unix_nano: u64,
    time_unix_nano: u64,
) -> Vec<u8> {
    let mut request_buf = Vec::new();
    // ResourceMetrics (field 1 of ExportMetricsServiceRequest)
    encode_message_field_in_place(&mut request_buf, 1, |buf| {
        // Resource (field 1 of ResourceMetrics)
        encode_message_field_in_place(buf, 1, |buf| {
            encode_resource(buf, resource_attrs);
        });
        // ScopeMetrics (field 2 of ResourceMetrics)
        encode_message_field_in_place(buf, 2, |buf| {
            // InstrumentationScope (field 1 of ScopeMetrics)
            encode_message_field_in_place(buf, 1, |buf| {
                encode_scope(buf, scope_name, scope_version);
            });
            // Metrics (field 2 of ScopeMetrics, repeated)
            for snapshot in snapshots {
                encode_message_field_in_place(buf, 2, |buf| {
                    encode_metric(buf, snapshot, start_time_unix_nano, time_unix_nano);
                });
            }
        });
    });
    request_buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn encode_counter_metric_is_nonempty() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "http_requests_total".to_string(),
            description: "Total HTTP requests".to_string(),
            data_points: vec![(
                Arc::new(vec![
                    ("method".to_string(), "GET".to_string()),
                    ("status".to_string(), "200".to_string()),
                ]),
                42,
                None,
            )],
        }];

        let bytes = encode_export_metrics_request(
            &[KeyValue {
                key: "service.name".to_string(),
                value: crate::otlp_trace::AnyValue::String("test-svc".to_string()),
            }],
            "rolly",
            "0.3.0",
            &snapshots,
            1_000_000_000,
            2_000_000_000,
        );

        assert!(!bytes.is_empty());
        assert_eq!(bytes[0], 0x0A); // field 1, wire type 2

        // Verify the metric name is in the output
        let name = b"http_requests_total";
        assert!(
            bytes.windows(name.len()).any(|w| w == name),
            "metric name not found in encoded bytes"
        );
    }

    #[test]
    fn encode_gauge_metric_is_nonempty() {
        let snapshots = vec![MetricSnapshot::Gauge {
            name: "cpu_usage".to_string(),
            description: "CPU usage percentage".to_string(),
            data_points: vec![(
                Arc::new(vec![("core".to_string(), "0".to_string())]),
                75.5,
                None,
            )],
        }];

        let bytes = encode_export_metrics_request(
            &[KeyValue {
                key: "service.name".to_string(),
                value: crate::otlp_trace::AnyValue::String("test-svc".to_string()),
            }],
            "rolly",
            "0.3.0",
            &snapshots,
            1_000_000_000,
            2_000_000_000,
        );

        assert!(!bytes.is_empty());
        // Verify gauge value is encoded (75.5 as f64 bits, little-endian)
        let val_bytes = 75.5_f64.to_bits().to_le_bytes();
        assert!(
            bytes.windows(8).any(|w| w == val_bytes),
            "gauge value not found in encoded bytes"
        );
    }

    #[test]
    fn encode_counter_value_is_correct() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "c".to_string(),
            description: String::new(),
            data_points: vec![(Arc::new(vec![]), 99, None)],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // as_int value 99 encoded as fixed64 LE
        let val_bytes = (99_i64 as u64).to_le_bytes();
        assert!(
            bytes.windows(8).any(|w| w == val_bytes),
            "counter value 99 not found in encoded bytes"
        );
    }

    #[test]
    fn encode_multiple_data_points() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "multi".to_string(),
            description: String::new(),
            data_points: vec![
                (Arc::new(vec![("k".to_string(), "a".to_string())]), 10, None),
                (Arc::new(vec![("k".to_string(), "b".to_string())]), 20, None),
            ],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // Both values should be present
        let val10 = (10_i64 as u64).to_le_bytes();
        let val20 = (20_i64 as u64).to_le_bytes();
        assert!(bytes.windows(8).any(|w| w == val10));
        assert!(bytes.windows(8).any(|w| w == val20));
    }

    #[test]
    fn encode_counter_has_cumulative_temporality() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "c".to_string(),
            description: String::new(),
            data_points: vec![(Arc::new(vec![]), 1, None)],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // aggregation_temporality = 2 (CUMULATIVE): varint field 2, value 2
        // tag = (2<<3)|0 = 0x10, value = 0x02
        assert!(
            bytes.windows(2).any(|w| w == [0x10, 0x02]),
            "CUMULATIVE temporality not found"
        );
    }

    #[test]
    fn encode_counter_is_monotonic() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "c".to_string(),
            description: String::new(),
            data_points: vec![(Arc::new(vec![]), 1, None)],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // is_monotonic = true: varint field 3, value 1
        // tag = (3<<3)|0 = 0x18, value = 0x01
        assert!(
            bytes.windows(2).any(|w| w == [0x18, 0x01]),
            "is_monotonic=true not found"
        );
    }

    #[test]
    fn encode_mixed_counter_and_gauge() {
        let snapshots = vec![
            MetricSnapshot::Counter {
                name: "requests".to_string(),
                description: String::new(),
                data_points: vec![(Arc::new(vec![]), 100, None)],
            },
            MetricSnapshot::Gauge {
                name: "temperature".to_string(),
                description: String::new(),
                data_points: vec![(Arc::new(vec![]), 36.6, None)],
            },
        ];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        assert!(bytes.windows(8).any(|w| w == b"requests"));
        assert!(bytes.windows(11).any(|w| w == b"temperature"));
    }

    #[test]
    fn encode_histogram_metric_is_nonempty() {
        let snapshots = vec![MetricSnapshot::Histogram {
            name: "request_duration".to_string(),
            description: "Request duration histogram".to_string(),
            boundaries: vec![10.0, 50.0, 100.0],
            data_points: vec![crate::metrics::HistogramDataPoint {
                attrs: Arc::new(vec![("method".to_string(), "GET".to_string())]),
                bucket_counts: vec![5, 10, 3, 2],
                sum: 1234.5,
                count: 20,
                min: 1.0,
                max: 250.0,
                exemplar: None,
            }],
        }];

        let bytes = encode_export_metrics_request(
            &[KeyValue {
                key: "service.name".to_string(),
                value: crate::otlp_trace::AnyValue::String("test-svc".to_string()),
            }],
            "rolly",
            "0.3.0",
            &snapshots,
            1_000_000_000,
            2_000_000_000,
        );

        assert!(!bytes.is_empty());
        let name = b"request_duration";
        assert!(
            bytes.windows(name.len()).any(|w| w == name),
            "metric name not found in encoded bytes"
        );
    }

    #[test]
    fn encode_histogram_has_cumulative_temporality() {
        let snapshots = vec![MetricSnapshot::Histogram {
            name: "h".to_string(),
            description: String::new(),
            boundaries: vec![10.0],
            data_points: vec![crate::metrics::HistogramDataPoint {
                attrs: Arc::new(vec![]),
                bucket_counts: vec![1, 0],
                sum: 5.0,
                count: 1,
                min: 5.0,
                max: 5.0,
                exemplar: None,
            }],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // aggregation_temporality = 2 (CUMULATIVE): tag = 0x10, value = 0x02
        assert!(
            bytes.windows(2).any(|w| w == [0x10, 0x02]),
            "CUMULATIVE temporality not found"
        );
    }

    #[test]
    fn encode_histogram_bucket_counts_present() {
        let snapshots = vec![MetricSnapshot::Histogram {
            name: "h".to_string(),
            description: String::new(),
            boundaries: vec![10.0],
            data_points: vec![crate::metrics::HistogramDataPoint {
                attrs: Arc::new(vec![]),
                bucket_counts: vec![3, 7],
                sum: 100.0,
                count: 10,
                min: 1.0,
                max: 50.0,
                exemplar: None,
            }],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // bucket_counts contains 3 and 7 as fixed64 LE
        let val3 = 3u64.to_le_bytes();
        let val7 = 7u64.to_le_bytes();
        assert!(bytes.windows(8).any(|w| w == val3));
        assert!(bytes.windows(8).any(|w| w == val7));
    }

    #[test]
    fn encode_histogram_attributes_present() {
        let snapshots = vec![MetricSnapshot::Histogram {
            name: "h".to_string(),
            description: String::new(),
            boundaries: vec![10.0],
            data_points: vec![crate::metrics::HistogramDataPoint {
                attrs: Arc::new(vec![("method".to_string(), "GET".to_string())]),
                bucket_counts: vec![1, 0],
                sum: 5.0,
                count: 1,
                min: 5.0,
                max: 5.0,
                exemplar: None,
            }],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        assert!(bytes.windows(6).any(|w| w == b"method"));
        assert!(bytes.windows(3).any(|w| w == b"GET"));
    }

    #[test]
    fn encode_mixed_counter_gauge_histogram() {
        let snapshots = vec![
            MetricSnapshot::Counter {
                name: "requests".to_string(),
                description: String::new(),
                data_points: vec![(Arc::new(vec![]), 100, None)],
            },
            MetricSnapshot::Gauge {
                name: "temperature".to_string(),
                description: String::new(),
                data_points: vec![(Arc::new(vec![]), 36.6, None)],
            },
            MetricSnapshot::Histogram {
                name: "latency".to_string(),
                description: String::new(),
                boundaries: vec![10.0],
                data_points: vec![crate::metrics::HistogramDataPoint {
                    attrs: Arc::new(vec![]),
                    bucket_counts: vec![1, 1],
                    sum: 15.0,
                    count: 2,
                    min: 5.0,
                    max: 10.0,
                    exemplar: None,
                }],
            },
        ];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        assert!(bytes.windows(8).any(|w| w == b"requests"));
        assert!(bytes.windows(11).any(|w| w == b"temperature"));
        assert!(bytes.windows(7).any(|w| w == b"latency"));
    }

    #[test]
    fn encode_attributes_in_data_point() {
        let snapshots = vec![MetricSnapshot::Counter {
            name: "c".to_string(),
            description: String::new(),
            data_points: vec![(
                Arc::new(vec![("method".to_string(), "GET".to_string())]),
                1,
                None,
            )],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        assert!(bytes.windows(6).any(|w| w == b"method"));
        assert!(bytes.windows(3).any(|w| w == b"GET"));
    }

    #[test]
    fn encode_counter_with_exemplar() {
        let exemplar = Some(crate::metrics::Exemplar {
            trace_id: [0x01; 16],
            span_id: [0x02; 8],
            time_unix_nano: 5_000_000_000,
            value: crate::metrics::ExemplarValue::Int(42),
        });
        let snapshots = vec![MetricSnapshot::Counter {
            name: "c".to_string(),
            description: String::new(),
            data_points: vec![(Arc::new(vec![]), 42, exemplar)],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // trace_id bytes should be present
        assert!(bytes.windows(16).any(|w| w == [0x01; 16]));
        // span_id bytes should be present
        assert!(bytes.windows(8).any(|w| w == [0x02; 8]));
    }

    #[test]
    fn encode_histogram_with_exemplar() {
        let exemplar = Some(crate::metrics::Exemplar {
            trace_id: [0xAA; 16],
            span_id: [0xBB; 8],
            time_unix_nano: 9_000_000_000,
            value: crate::metrics::ExemplarValue::Double(42.5),
        });
        let snapshots = vec![MetricSnapshot::Histogram {
            name: "h".to_string(),
            description: String::new(),
            boundaries: vec![10.0],
            data_points: vec![crate::metrics::HistogramDataPoint {
                attrs: Arc::new(vec![]),
                bucket_counts: vec![1, 0],
                sum: 42.5,
                count: 1,
                min: 42.5,
                max: 42.5,
                exemplar,
            }],
        }];

        let bytes = encode_export_metrics_request(&[], "rolly", "0.3.0", &snapshots, 0, 0);

        // trace_id and span_id bytes should be present
        assert!(bytes.windows(16).any(|w| w == [0xAA; 16]));
        assert!(bytes.windows(8).any(|w| w == [0xBB; 8]));
    }
}

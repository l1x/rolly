#![cfg(feature = "_bench")]

//! Wire compatibility tests: encode with ro11y's hand-rolled protobuf,
//! decode with prost / opentelemetry-proto, and assert field-level correctness.

use prost::Message;

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;

use ro11y::bench::{
    encode_export_logs_request, encode_export_metrics_request, encode_export_trace_request,
    AnyValue, Exemplar, ExemplarValue, HistogramDataPoint, KeyValue, LogData, MetricSnapshot,
    SeverityNumber, SpanData, SpanKind, SpanStatus, StatusCode,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resource_attrs() -> Vec<KeyValue> {
    vec![
        KeyValue {
            key: "service.name".into(),
            value: AnyValue::String("test-svc".into()),
        },
        KeyValue {
            key: "service.version".into(),
            value: AnyValue::String("1.0.0".into()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Trace tests
// ---------------------------------------------------------------------------

#[test]
fn trace_request_decodes_via_prost() {
    let span = SpanData {
        trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        span_id: [0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8],
        parent_span_id: [0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8],
        name: "my-span".into(),
        kind: SpanKind::Server,
        start_time_unix_nano: 1_000_000_000,
        end_time_unix_nano: 2_000_000_000,
        attributes: vec![
            KeyValue {
                key: "http.method".into(),
                value: AnyValue::String("GET".into()),
            },
            KeyValue {
                key: "http.status_code".into(),
                value: AnyValue::Int(200),
            },
            KeyValue {
                key: "http.success".into(),
                value: AnyValue::Bool(true),
            },
            KeyValue {
                key: "http.latency".into(),
                value: AnyValue::Double(1.5),
            },
            KeyValue {
                key: "http.body".into(),
                value: AnyValue::Bytes(vec![0xDE, 0xAD]),
            },
        ],
        status: Some(SpanStatus {
            message: "all good".into(),
            code: StatusCode::Ok,
        }),
    };

    let bytes = encode_export_trace_request(&resource_attrs(), "ro11y", "1.0.0", &[span]);
    let req = ExportTraceServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    assert_eq!(req.resource_spans.len(), 1);
    let rs = &req.resource_spans[0];

    // Resource attributes
    let res = rs.resource.as_ref().expect("resource missing");
    let svc_name = res
        .attributes
        .iter()
        .find(|a| a.key == "service.name")
        .expect("service.name missing");
    assert_eq!(
        svc_name.value.as_ref().unwrap().value,
        Some(
            opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                "test-svc".into()
            )
        )
    );

    // Scope
    assert_eq!(rs.scope_spans.len(), 1);
    let ss = &rs.scope_spans[0];
    let scope = ss.scope.as_ref().expect("scope missing");
    assert_eq!(scope.name, "ro11y");
    assert_eq!(scope.version, "1.0.0");

    // Span
    assert_eq!(ss.spans.len(), 1);
    let s = &ss.spans[0];
    assert_eq!(
        s.trace_id,
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_eq!(
        s.span_id,
        vec![0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8]
    );
    assert_eq!(
        s.parent_span_id,
        vec![0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8]
    );
    assert_eq!(s.name, "my-span");
    assert_eq!(s.kind, 2); // SPAN_KIND_SERVER
    assert_eq!(s.start_time_unix_nano, 1_000_000_000);
    assert_eq!(s.end_time_unix_nano, 2_000_000_000);

    // Attributes
    assert_eq!(s.attributes.len(), 5);
    let find_attr = |key: &str| s.attributes.iter().find(|a| a.key == key).unwrap();

    let method = find_attr("http.method");
    assert_eq!(
        method.value.as_ref().unwrap().value,
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue("GET".into()))
    );

    let status_code = find_attr("http.status_code");
    assert_eq!(
        status_code.value.as_ref().unwrap().value,
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(200))
    );

    let success = find_attr("http.success");
    assert_eq!(
        success.value.as_ref().unwrap().value,
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(true))
    );

    let latency = find_attr("http.latency");
    assert_eq!(
        latency.value.as_ref().unwrap().value,
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::DoubleValue(1.5))
    );

    let body = find_attr("http.body");
    assert_eq!(
        body.value.as_ref().unwrap().value,
        Some(
            opentelemetry_proto::tonic::common::v1::any_value::Value::BytesValue(vec![0xDE, 0xAD])
        )
    );

    // Status
    let status = s.status.as_ref().expect("status missing");
    assert_eq!(status.message, "all good");
    assert_eq!(status.code, 1); // STATUS_CODE_OK
}

#[test]
fn trace_empty_attributes_decode() {
    let span = SpanData {
        trace_id: [0xAA; 16],
        span_id: [0xBB; 8],
        parent_span_id: [0; 8],
        name: "no-attrs".into(),
        kind: SpanKind::Internal,
        start_time_unix_nano: 100,
        end_time_unix_nano: 200,
        attributes: vec![],
        status: None,
    };

    let bytes = encode_export_trace_request(&[], "ro11y", "1.0.0", &[span]);
    let req = ExportTraceServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let s = &req.resource_spans[0].scope_spans[0].spans[0];
    assert_eq!(s.name, "no-attrs");
    assert!(s.attributes.is_empty());
}

#[test]
fn trace_multiple_spans_decode() {
    let spans: Vec<SpanData> = (0..3)
        .map(|i| SpanData {
            trace_id: [i as u8; 16],
            span_id: [i as u8 + 10; 8],
            parent_span_id: [0; 8],
            name: format!("span-{}", i),
            kind: SpanKind::Client,
            start_time_unix_nano: 1000 * (i as u64 + 1),
            end_time_unix_nano: 2000 * (i as u64 + 1),
            attributes: vec![],
            status: None,
        })
        .collect();

    let bytes = encode_export_trace_request(&resource_attrs(), "ro11y", "1.0.0", &spans);
    let req = ExportTraceServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let decoded_spans = &req.resource_spans[0].scope_spans[0].spans;
    assert_eq!(decoded_spans.len(), 3);
    for (i, s) in decoded_spans.iter().enumerate() {
        assert_eq!(s.name, format!("span-{}", i));
    }
}

// ---------------------------------------------------------------------------
// Log tests
// ---------------------------------------------------------------------------

#[test]
fn log_request_decodes_via_prost() {
    let log = LogData {
        time_unix_nano: 5_000_000_000,
        severity_number: SeverityNumber::Warn,
        severity_text: "WARN".into(),
        body: AnyValue::String("something happened".into()),
        attributes: vec![KeyValue {
            key: "module".into(),
            value: AnyValue::String("auth".into()),
        }],
        trace_id: [0x11; 16],
        span_id: [0x22; 8],
    };

    let bytes = encode_export_logs_request(&resource_attrs(), "ro11y", "1.0.0", &[log]);
    let req = ExportLogsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    assert_eq!(req.resource_logs.len(), 1);
    let rl = &req.resource_logs[0];
    let sl = &rl.scope_logs[0];
    let scope = sl.scope.as_ref().unwrap();
    assert_eq!(scope.name, "ro11y");

    assert_eq!(sl.log_records.len(), 1);
    let lr = &sl.log_records[0];
    assert_eq!(lr.time_unix_nano, 5_000_000_000);
    assert_eq!(lr.severity_number, 13); // WARN = 13
    assert_eq!(lr.severity_text, "WARN");

    // Body
    let body_val = lr.body.as_ref().unwrap().value.as_ref().unwrap();
    assert_eq!(
        *body_val,
        opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
            "something happened".into()
        )
    );

    // Attributes
    assert_eq!(lr.attributes.len(), 1);
    assert_eq!(lr.attributes[0].key, "module");

    // Trace context
    assert_eq!(lr.trace_id, vec![0x11; 16]);
    assert_eq!(lr.span_id, vec![0x22; 8]);
}

#[test]
fn log_all_severity_levels() {
    let levels = [
        (SeverityNumber::Trace, 1),
        (SeverityNumber::Debug, 5),
        (SeverityNumber::Info, 9),
        (SeverityNumber::Warn, 13),
        (SeverityNumber::Error, 17),
        (SeverityNumber::Fatal, 21),
    ];

    for (sev, expected_num) in levels {
        let log = LogData {
            time_unix_nano: 1_000_000,
            severity_number: sev,
            severity_text: format!("{:?}", sev),
            body: AnyValue::String("msg".into()),
            attributes: vec![],
            trace_id: [0; 16],
            span_id: [0; 8],
        };

        let bytes = encode_export_logs_request(&[], "ro11y", "1.0.0", &[log]);
        let req = ExportLogsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

        let lr = &req.resource_logs[0].scope_logs[0].log_records[0];
        assert_eq!(
            lr.severity_number, expected_num,
            "severity {:?} should be {}",
            sev, expected_num
        );
    }
}

// ---------------------------------------------------------------------------
// Metrics tests
// ---------------------------------------------------------------------------

#[test]
fn counter_metric_decodes_via_prost() {
    let snapshots = vec![MetricSnapshot::Counter {
        name: "http_requests_total".into(),
        description: "Total HTTP requests".into(),
        data_points: vec![(
            vec![
                ("method".into(), "GET".into()),
                ("status".into(), "200".into()),
            ],
            42,
            None,
        )],
    }];

    let bytes = encode_export_metrics_request(
        &resource_attrs(),
        "ro11y",
        "1.0.0",
        &snapshots,
        1_000_000_000,
        2_000_000_000,
    );
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    assert_eq!(req.resource_metrics.len(), 1);
    let rm = &req.resource_metrics[0];
    let sm = &rm.scope_metrics[0];
    assert_eq!(sm.metrics.len(), 1);

    let m = &sm.metrics[0];
    assert_eq!(m.name, "http_requests_total");
    assert_eq!(m.description, "Total HTTP requests");

    // Sum
    let sum = m.data.as_ref().unwrap();
    match sum {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(s) => {
            assert!(s.is_monotonic);
            assert_eq!(s.aggregation_temporality, 2); // CUMULATIVE
            assert_eq!(s.data_points.len(), 1);
            let dp = &s.data_points[0];
            assert_eq!(
                dp.value,
                Some(opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsInt(42))
            );
            // Attributes
            assert_eq!(dp.attributes.len(), 2);
        }
        other => panic!("expected Sum, got {:?}", other),
    }
}

#[test]
fn gauge_metric_decodes_via_prost() {
    let snapshots = vec![MetricSnapshot::Gauge {
        name: "cpu_usage".into(),
        description: "CPU usage".into(),
        data_points: vec![(vec![("core".into(), "0".into())], 75.5, None)],
    }];

    let bytes = encode_export_metrics_request(
        &resource_attrs(),
        "ro11y",
        "1.0.0",
        &snapshots,
        1_000_000_000,
        2_000_000_000,
    );
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let m = &req.resource_metrics[0].scope_metrics[0].metrics[0];
    assert_eq!(m.name, "cpu_usage");

    match m.data.as_ref().unwrap() {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Gauge(g) => {
            assert_eq!(g.data_points.len(), 1);
            let dp = &g.data_points[0];
            assert_eq!(
                dp.value,
                Some(
                    opentelemetry_proto::tonic::metrics::v1::number_data_point::Value::AsDouble(
                        75.5
                    )
                )
            );
        }
        other => panic!("expected Gauge, got {:?}", other),
    }
}

#[test]
fn histogram_metric_decodes_via_prost() {
    let snapshots = vec![MetricSnapshot::Histogram {
        name: "request_duration_ms".into(),
        description: "Request duration in milliseconds".into(),
        boundaries: vec![10.0, 50.0, 100.0, 500.0],
        data_points: vec![HistogramDataPoint {
            attrs: vec![
                ("method".into(), "GET".into()),
                ("path".into(), "/api".into()),
            ],
            bucket_counts: vec![5, 15, 8, 3, 1],
            sum: 2345.6,
            count: 32,
            min: 2.1,
            max: 750.0,
            exemplar: None,
        }],
    }];

    let bytes = encode_export_metrics_request(
        &resource_attrs(),
        "ro11y",
        "1.0.0",
        &snapshots,
        1_000_000_000,
        2_000_000_000,
    );
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let m = &req.resource_metrics[0].scope_metrics[0].metrics[0];
    assert_eq!(m.name, "request_duration_ms");
    assert_eq!(m.description, "Request duration in milliseconds");

    match m.data.as_ref().unwrap() {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(h) => {
            assert_eq!(h.aggregation_temporality, 2); // CUMULATIVE
            assert_eq!(h.data_points.len(), 1);

            let dp = &h.data_points[0];
            assert_eq!(dp.count, 32);
            assert!((dp.sum.unwrap() - 2345.6).abs() < f64::EPSILON);
            assert_eq!(dp.bucket_counts, vec![5, 15, 8, 3, 1]);
            assert_eq!(dp.explicit_bounds, vec![10.0, 50.0, 100.0, 500.0]);
            assert!((dp.min.unwrap() - 2.1).abs() < f64::EPSILON);
            assert!((dp.max.unwrap() - 750.0).abs() < f64::EPSILON);

            // Attributes
            assert_eq!(dp.attributes.len(), 2);
            let attr_keys: Vec<&str> = dp.attributes.iter().map(|a| a.key.as_str()).collect();
            assert!(attr_keys.contains(&"method"));
            assert!(attr_keys.contains(&"path"));
        }
        other => panic!("expected Histogram, got {:?}", other),
    }
}

#[test]
fn histogram_multiple_data_points_decode() {
    let snapshots = vec![MetricSnapshot::Histogram {
        name: "latency".into(),
        description: String::new(),
        boundaries: vec![10.0, 100.0],
        data_points: vec![
            HistogramDataPoint {
                attrs: vec![("method".into(), "GET".into())],
                bucket_counts: vec![5, 3, 1],
                sum: 200.0,
                count: 9,
                min: 1.0,
                max: 150.0,
                exemplar: None,
            },
            HistogramDataPoint {
                attrs: vec![("method".into(), "POST".into())],
                bucket_counts: vec![2, 8, 0],
                sum: 450.0,
                count: 10,
                min: 3.0,
                max: 95.0,
                exemplar: None,
            },
        ],
    }];

    let bytes =
        encode_export_metrics_request(&resource_attrs(), "ro11y", "1.0.0", &snapshots, 0, 0);
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    match req.resource_metrics[0].scope_metrics[0].metrics[0]
        .data
        .as_ref()
        .unwrap()
    {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(h) => {
            assert_eq!(h.data_points.len(), 2);
        }
        other => panic!("expected Histogram, got {:?}", other),
    }
}

#[test]
fn mixed_metrics_decode() {
    let snapshots = vec![
        MetricSnapshot::Counter {
            name: "requests".into(),
            description: String::new(),
            data_points: vec![(vec![], 100, None)],
        },
        MetricSnapshot::Gauge {
            name: "temperature".into(),
            description: String::new(),
            data_points: vec![(vec![], 36.6, None)],
        },
        MetricSnapshot::Histogram {
            name: "latency".into(),
            description: String::new(),
            boundaries: vec![10.0],
            data_points: vec![HistogramDataPoint {
                attrs: vec![],
                bucket_counts: vec![1, 1],
                sum: 15.0,
                count: 2,
                min: 5.0,
                max: 10.0,
                exemplar: None,
            }],
        },
    ];

    let bytes =
        encode_export_metrics_request(&resource_attrs(), "ro11y", "1.0.0", &snapshots, 0, 0);
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let metrics = &req.resource_metrics[0].scope_metrics[0].metrics;
    assert_eq!(metrics.len(), 3);

    let names: Vec<&str> = metrics.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"requests"));
    assert!(names.contains(&"temperature"));
    assert!(names.contains(&"latency"));

    // Verify types
    for m in metrics {
        match m.name.as_str() {
            "requests" => assert!(matches!(
                m.data,
                Some(opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(
                    _
                ))
            )),
            "temperature" => assert!(matches!(
                m.data,
                Some(opentelemetry_proto::tonic::metrics::v1::metric::Data::Gauge(_))
            )),
            "latency" => assert!(matches!(
                m.data,
                Some(opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(_))
            )),
            other => panic!("unexpected metric: {}", other),
        }
    }
}

#[test]
fn counter_with_exemplar_decodes_via_prost() {
    let exemplar = Some(Exemplar {
        trace_id: [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ],
        span_id: [0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8],
        time_unix_nano: 5_000_000_000,
        value: ExemplarValue::Int(42),
    });
    let snapshots = vec![MetricSnapshot::Counter {
        name: "exemplar_counter".into(),
        description: String::new(),
        data_points: vec![(vec![], 42, exemplar)],
    }];

    let bytes = encode_export_metrics_request(
        &resource_attrs(),
        "ro11y",
        "1.0.0",
        &snapshots,
        1_000_000_000,
        2_000_000_000,
    );
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let m = &req.resource_metrics[0].scope_metrics[0].metrics[0];
    match m.data.as_ref().unwrap() {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Sum(s) => {
            let dp = &s.data_points[0];
            assert_eq!(dp.exemplars.len(), 1);
            let ex = &dp.exemplars[0];
            assert_eq!(
                ex.trace_id,
                vec![
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
                    0x0E, 0x0F, 0x10
                ]
            );
            assert_eq!(
                ex.span_id,
                vec![0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8]
            );
            assert_eq!(ex.time_unix_nano, 5_000_000_000);
            assert_eq!(
                ex.value,
                Some(opentelemetry_proto::tonic::metrics::v1::exemplar::Value::AsInt(42))
            );
        }
        other => panic!("expected Sum, got {:?}", other),
    }
}

#[test]
fn histogram_with_exemplar_decodes_via_prost() {
    let exemplar = Some(Exemplar {
        trace_id: [0xAA; 16],
        span_id: [0xBB; 8],
        time_unix_nano: 9_000_000_000,
        value: ExemplarValue::Double(42.5),
    });
    let snapshots = vec![MetricSnapshot::Histogram {
        name: "exemplar_hist".into(),
        description: String::new(),
        boundaries: vec![10.0],
        data_points: vec![HistogramDataPoint {
            attrs: vec![],
            bucket_counts: vec![1, 0],
            sum: 42.5,
            count: 1,
            min: 42.5,
            max: 42.5,
            exemplar,
        }],
    }];

    let bytes = encode_export_metrics_request(
        &resource_attrs(),
        "ro11y",
        "1.0.0",
        &snapshots,
        1_000_000_000,
        2_000_000_000,
    );
    let req = ExportMetricsServiceRequest::decode(bytes.as_slice()).expect("decode failed");

    let m = &req.resource_metrics[0].scope_metrics[0].metrics[0];
    match m.data.as_ref().unwrap() {
        opentelemetry_proto::tonic::metrics::v1::metric::Data::Histogram(h) => {
            let dp = &h.data_points[0];
            assert_eq!(dp.exemplars.len(), 1);
            let ex = &dp.exemplars[0];
            assert_eq!(ex.trace_id, vec![0xAA; 16]);
            assert_eq!(ex.span_id, vec![0xBB; 8]);
            assert_eq!(ex.time_unix_nano, 9_000_000_000);
            assert_eq!(
                ex.value,
                Some(opentelemetry_proto::tonic::metrics::v1::exemplar::Value::AsDouble(42.5))
            );
        }
        other => panic!("expected Histogram, got {:?}", other),
    }
}

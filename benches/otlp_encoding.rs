use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rolly::bench::*;

fn test_span() -> SpanData {
    SpanData {
        trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        span_id: [1, 2, 3, 4, 5, 6, 7, 8],
        parent_span_id: [0; 8],
        name: "GET /api/v1/users".to_string(),
        kind: SpanKind::Server,
        start_time_unix_nano: 1_700_000_000_000_000_000,
        end_time_unix_nano: 1_700_000_000_050_000_000,
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

fn test_span_with_attrs(n: usize) -> SpanData {
    let mut span = test_span();
    span.attributes = (0..n)
        .map(|i| KeyValue {
            key: format!("attr.key.{}", i),
            value: AnyValue::String(format!("value-{}", i)),
        })
        .collect();
    span
}

fn test_log() -> LogData {
    LogData {
        time_unix_nano: 1_700_000_000_000_000_000,
        severity_number: SeverityNumber::Info,
        severity_text: "INFO".to_string(),
        body: AnyValue::String("request completed successfully".to_string()),
        attributes: vec![KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String("test-svc".to_string()),
        }],
        trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        span_id: [1, 2, 3, 4, 5, 6, 7, 8],
    }
}

fn resource_attrs() -> Vec<KeyValue> {
    vec![
        KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String("bench-svc".to_string()),
        },
        KeyValue {
            key: "service.version".to_string(),
            value: AnyValue::String("0.1.0".to_string()),
        },
        KeyValue {
            key: "deployment.environment".to_string(),
            value: AnyValue::String("production".to_string()),
        },
    ]
}

fn bench_encode_trace_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_export_trace_request");
    let attrs = resource_attrs();

    let spans_1: Vec<SpanData> = vec![test_span()];
    let spans_10: Vec<SpanData> = (0..10).map(|_| test_span()).collect();
    let spans_100: Vec<SpanData> = (0..100).map(|_| test_span()).collect();

    group.bench_function("1_span", |b| {
        b.iter(|| {
            black_box(encode_export_trace_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&spans_1),
            ));
        });
    });

    group.bench_function("10_spans", |b| {
        b.iter(|| {
            black_box(encode_export_trace_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&spans_10),
            ));
        });
    });

    group.bench_function("100_spans", |b| {
        b.iter(|| {
            black_box(encode_export_trace_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&spans_100),
            ));
        });
    });

    group.finish();
}

fn bench_encode_logs_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_export_logs_request");
    let attrs = resource_attrs();

    let logs_1: Vec<LogData> = vec![test_log()];
    let logs_10: Vec<LogData> = (0..10).map(|_| test_log()).collect();
    let logs_100: Vec<LogData> = (0..100).map(|_| test_log()).collect();

    group.bench_function("1_log", |b| {
        b.iter(|| {
            black_box(encode_export_logs_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&logs_1),
            ));
        });
    });

    group.bench_function("10_logs", |b| {
        b.iter(|| {
            black_box(encode_export_logs_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&logs_10),
            ));
        });
    });

    group.bench_function("100_logs", |b| {
        b.iter(|| {
            black_box(encode_export_logs_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&logs_100),
            ));
        });
    });

    group.finish();
}

fn bench_attribute_heavy_spans(c: &mut Criterion) {
    let mut group = c.benchmark_group("attribute_heavy_spans");
    let attrs = resource_attrs();

    let spans_1_attr: Vec<SpanData> = (0..10).map(|_| test_span_with_attrs(1)).collect();
    let spans_10_attrs: Vec<SpanData> = (0..10).map(|_| test_span_with_attrs(10)).collect();

    group.bench_function("10_spans_1_attr_each", |b| {
        b.iter(|| {
            black_box(encode_export_trace_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&spans_1_attr),
            ));
        });
    });

    group.bench_function("10_spans_10_attrs_each", |b| {
        b.iter(|| {
            black_box(encode_export_trace_request(
                black_box(&attrs),
                "rolly",
                "0.2.0",
                black_box(&spans_10_attrs),
            ));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_trace_request,
    bench_encode_logs_request,
    bench_attribute_heavy_spans,
);
criterion_main!(benches);

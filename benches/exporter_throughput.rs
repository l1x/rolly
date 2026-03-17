use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rolly::bench::*;

fn make_payload(size: usize) -> Vec<u8> {
    let span = SpanData {
        trace_id: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        span_id: [1, 2, 3, 4, 5, 6, 7, 8],
        parent_span_id: [0; 8],
        name: "GET /api/v1/users".to_string(),
        kind: SpanKind::Server,
        start_time_unix_nano: 1_700_000_000_000_000_000,
        end_time_unix_nano: 1_700_000_000_050_000_000,
        attributes: (0..size)
            .map(|i| KeyValue {
                key: format!("attr.{}", i),
                value: AnyValue::String(format!("val-{}", i)),
            })
            .collect(),
        status: Some(SpanStatus {
            message: String::new(),
            code: StatusCode::Ok,
        }),
    };
    let attrs = vec![KeyValue {
        key: "service.name".to_string(),
        value: AnyValue::String("bench-svc".to_string()),
    }];
    encode_export_trace_request(&attrs, "rolly", "0.2.0", &[span])
}

fn bench_send_traces(c: &mut Criterion) {
    let mut group = c.benchmark_group("send_traces");
    let payload = make_payload(3);

    group.bench_function("try_send_1", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        b.iter(|| {
            rt.block_on(async {
                let (exporter, _rx) =
                    Exporter::start_test_with_capacity(1024, BackpressureStrategy::Drop);
                exporter.send_traces(black_box(payload.clone()));
            });
        });
    });

    group.bench_function("try_send_100", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        b.iter(|| {
            rt.block_on(async {
                let (exporter, _rx) =
                    Exporter::start_test_with_capacity(1024, BackpressureStrategy::Drop);
                for _ in 0..100 {
                    exporter.send_traces(black_box(payload.clone()));
                }
            });
        });
    });

    group.finish();
}

fn bench_batch_concatenation(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_concatenation");
    let payload = Bytes::from(make_payload(3));

    group.bench_function("concat_10_payloads", |b| {
        let batch: Vec<Bytes> = (0..10).map(|_| payload.clone()).collect();
        b.iter(|| {
            let total_len: usize = batch.iter().map(|b| b.len()).sum();
            let mut buf = Vec::with_capacity(total_len);
            for item in black_box(&batch) {
                buf.extend_from_slice(item);
            }
            black_box(&buf);
        });
    });

    group.bench_function("concat_100_payloads", |b| {
        let batch: Vec<Bytes> = (0..100).map(|_| payload.clone()).collect();
        b.iter(|| {
            let total_len: usize = batch.iter().map(|b| b.len()).sum();
            let mut buf = Vec::with_capacity(total_len);
            for item in black_box(&batch) {
                buf.extend_from_slice(item);
            }
            black_box(&buf);
        });
    });

    group.bench_function("concat_1000_payloads", |b| {
        let batch: Vec<Bytes> = (0..1000).map(|_| payload.clone()).collect();
        b.iter(|| {
            let total_len: usize = batch.iter().map(|b| b.len()).sum();
            let mut buf = Vec::with_capacity(total_len);
            for item in black_box(&batch) {
                buf.extend_from_slice(item);
            }
            black_box(&buf);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_send_traces, bench_batch_concatenation,);
criterion_main!(benches);

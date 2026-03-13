use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rolly::bench::*;

fn resource_attrs() -> Vec<KeyValue> {
    vec![
        KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String("bench-svc".to_string()),
        },
        KeyValue {
            key: "service.version".to_string(),
            value: AnyValue::String("0.3.0".to_string()),
        },
        KeyValue {
            key: "deployment.environment".to_string(),
            value: AnyValue::String("production".to_string()),
        },
    ]
}

fn make_counter_snapshots(n_metrics: usize, n_data_points: usize) -> Vec<MetricSnapshot> {
    (0..n_metrics)
        .map(|i| MetricSnapshot::Counter {
            name: format!("http_requests_total_{}", i),
            description: format!("Total HTTP requests {}", i),
            data_points: (0..n_data_points)
                .map(|j| {
                    (
                        Arc::new(vec![
                            ("method".to_string(), format!("M{}", j % 4)),
                            ("status".to_string(), format!("{}", 200 + j % 5)),
                        ]),
                        (100 + j * 10) as i64,
                        None,
                    )
                })
                .collect(),
        })
        .collect()
}

fn make_gauge_snapshots(n_metrics: usize, n_data_points: usize) -> Vec<MetricSnapshot> {
    (0..n_metrics)
        .map(|i| MetricSnapshot::Gauge {
            name: format!("cpu_usage_{}", i),
            description: format!("CPU usage {}", i),
            data_points: (0..n_data_points)
                .map(|j| {
                    (
                        Arc::new(vec![("core".to_string(), format!("{}", j))]),
                        50.0 + j as f64 * 1.5,
                        None,
                    )
                })
                .collect(),
        })
        .collect()
}

fn make_histogram_snapshots(n_metrics: usize, n_data_points: usize) -> Vec<MetricSnapshot> {
    let boundaries = vec![5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0];
    (0..n_metrics)
        .map(|i| MetricSnapshot::Histogram {
            name: format!("request_duration_{}", i),
            description: format!("Request duration {}", i),
            boundaries: boundaries.clone(),
            data_points: (0..n_data_points)
                .map(|j| HistogramDataPoint {
                    attrs: Arc::new(vec![
                        ("method".to_string(), format!("M{}", j % 4)),
                        ("status".to_string(), format!("{}", 200 + j % 5)),
                    ]),
                    bucket_counts: vec![10, 20, 30, 25, 15, 8, 3, 1, 0],
                    sum: 5000.0 + j as f64 * 100.0,
                    count: 112,
                    min: 0.5,
                    max: 850.0,
                    exemplar: None,
                })
                .collect(),
        })
        .collect()
}

fn bench_encode_metrics_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_export_metrics_request");
    let attrs = resource_attrs();

    let snap_1c_1dp = make_counter_snapshots(1, 1);
    let snap_1c_10dp = make_counter_snapshots(1, 10);
    let snap_10c_10dp = make_counter_snapshots(10, 10);

    group.bench_function("1_counter_1_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_1c_1dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.bench_function("1_counter_10_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_1c_10dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.bench_function("10_counters_10_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_10c_10dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.finish();
}

fn bench_encode_gauge_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_gauge_metrics");
    let attrs = resource_attrs();

    let snap_1g_4dp = make_gauge_snapshots(1, 4);
    let snap_10g_4dp = make_gauge_snapshots(10, 4);

    group.bench_function("1_gauge_4_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_1g_4dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.bench_function("10_gauges_4_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_10g_4dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.finish();
}

fn bench_encode_histogram_request(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_histogram_metrics");
    let attrs = resource_attrs();

    let snap_1h_1dp = make_histogram_snapshots(1, 1);
    let snap_1h_10dp = make_histogram_snapshots(1, 10);
    let snap_10h_10dp = make_histogram_snapshots(10, 10);

    group.bench_function("1_histogram_1_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_1h_1dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.bench_function("1_histogram_10_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_1h_10dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.bench_function("10_histograms_10_dp", |b| {
        b.iter(|| {
            black_box(encode_export_metrics_request(
                black_box(&attrs),
                "rolly",
                "0.3.0",
                black_box(&snap_10h_10dp),
                1_700_000_000_000_000_000,
                1_700_000_010_000_000_000,
            ));
        });
    });

    group.finish();
}

fn bench_histogram_observe(c: &mut Criterion) {
    let mut group = c.benchmark_group("histogram_observe");

    let registry = MetricsRegistry::new();
    let h = registry.histogram(
        "bench_histogram",
        "benchmark histogram",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );

    group.bench_function("no_attrs", |b| {
        b.iter(|| {
            h.observe(black_box(42.5), black_box(&[]));
        });
    });

    group.bench_function("2_attrs", |b| {
        b.iter(|| {
            h.observe(
                black_box(42.5),
                black_box(&[("method", "GET"), ("status", "200")]),
            );
        });
    });

    group.finish();
}

fn bench_counter_add(c: &mut Criterion) {
    let mut group = c.benchmark_group("counter_add");

    let registry = MetricsRegistry::new();
    let ctr = registry.counter("bench_counter", "benchmark counter");

    group.bench_function("no_attrs", |b| {
        b.iter(|| {
            ctr.add(black_box(1), black_box(&[]));
        });
    });

    group.bench_function("2_attrs", |b| {
        b.iter(|| {
            ctr.add(
                black_box(1),
                black_box(&[("method", "GET"), ("status", "200")]),
            );
        });
    });

    group.bench_function("5_attrs", |b| {
        b.iter(|| {
            ctr.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("status", "200"),
                    ("path", "/api/v1/users"),
                    ("host", "api.example.com"),
                    ("region", "us-east-1"),
                ]),
            );
        });
    });

    group.finish();
}

fn bench_gauge_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("gauge_set");

    let registry = MetricsRegistry::new();
    let g = registry.gauge("bench_gauge", "benchmark gauge");

    group.bench_function("no_attrs", |b| {
        b.iter(|| {
            g.set(black_box(42.5), black_box(&[]));
        });
    });

    group.bench_function("2_attrs", |b| {
        b.iter(|| {
            g.set(black_box(42.5), black_box(&[("core", "0"), ("numa", "0")]));
        });
    });

    group.finish();
}

fn bench_collect(c: &mut Criterion) {
    let mut group = c.benchmark_group("registry_collect");

    // Small registry: 5 counters, 3 label sets each
    let registry_small = MetricsRegistry::new();
    for i in 0..5 {
        let ctr = registry_small.counter(&format!("counter_{}", i), "bench");
        for j in 0..3 {
            ctr.add(1, &[("key", &format!("val{}", j))]);
        }
    }

    // Large registry: 50 counters + 20 gauges + 10 histograms, 10 label sets each
    let registry_large = MetricsRegistry::new();
    for i in 0..50 {
        let ctr = registry_large.counter(&format!("counter_{}", i), "bench");
        for j in 0..10 {
            ctr.add(1, &[("key", &format!("val{}", j))]);
        }
    }
    for i in 0..20 {
        let g = registry_large.gauge(&format!("gauge_{}", i), "bench");
        for j in 0..10 {
            g.set(j as f64, &[("key", &format!("val{}", j))]);
        }
    }
    let hist_boundaries = &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0];
    for i in 0..10 {
        let h = registry_large.histogram(&format!("histogram_{}", i), "bench", hist_boundaries);
        for j in 0..10 {
            h.observe(j as f64 * 10.0, &[("key", &format!("val{}", j))]);
        }
    }

    group.bench_function("5_counters_3_labels", |b| {
        b.iter(|| {
            black_box(registry_small.collect());
        });
    });

    group.bench_function("50_counters_20_gauges_10_histograms_10_labels", |b| {
        b.iter(|| {
            black_box(registry_large.collect());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_metrics_request,
    bench_encode_gauge_request,
    bench_encode_histogram_request,
    bench_counter_add,
    bench_gauge_set,
    bench_histogram_observe,
    bench_collect,
);
criterion_main!(benches);

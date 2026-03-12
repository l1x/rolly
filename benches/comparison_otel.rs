//! Head-to-head benchmark: ro11y vs opentelemetry_sdk 0.31
//!
//! Compares identical metric operations on the same hardware to produce
//! an apples-to-apples comparison.
//!
//! Run: `cargo bench --features _bench -- comparison`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ---------------------------------------------------------------------------
// ro11y setup
// ---------------------------------------------------------------------------
use ro11y::bench::*;

fn ro11y_registry() -> MetricsRegistry {
    MetricsRegistry::new()
}

// ---------------------------------------------------------------------------
// OpenTelemetry SDK setup
// ---------------------------------------------------------------------------
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::{KeyValue, metrics::Meter};
use opentelemetry_sdk::metrics::{ManualReader, SdkMeterProvider};

fn otel_provider() -> SdkMeterProvider {
    SdkMeterProvider::builder()
        .with_reader(ManualReader::builder().build())
        .build()
}

fn otel_meter(provider: &SdkMeterProvider) -> Meter {
    provider.meter("bench")
}

// ---------------------------------------------------------------------------
// Counter benchmarks
// ---------------------------------------------------------------------------

fn bench_counter_3_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_3_attrs");

    // ro11y
    let r_reg = ro11y_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    // warm up the attribute set
    r_ctr.add(1, &[("method", "GET"), ("status", "200"), ("region", "us-east-1")]);

    group.bench_function("ro11y", |b| {
        b.iter(|| {
            r_ctr.add(
                black_box(1),
                black_box(&[("method", "GET"), ("status", "200"), ("region", "us-east-1")]),
            );
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_ctr = o_meter.u64_counter("requests").build();
    // warm up
    o_ctr.add(1, &[
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ]);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_ctr.add(
                black_box(1),
                black_box(&[
                    KeyValue::new("method", "GET"),
                    KeyValue::new("status", "200"),
                    KeyValue::new("region", "us-east-1"),
                ]),
            );
        });
    });

    group.finish();
}

fn bench_counter_5_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_5_attrs");

    let attrs_ro11y: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
    ];

    // ro11y
    let r_reg = ro11y_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    r_ctr.add(1, attrs_ro11y);

    group.bench_function("ro11y", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(attrs_ro11y));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_ctr = o_meter.u64_counter("requests").build();
    o_ctr.add(1, &attrs_otel);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_ctr.add(black_box(1), black_box(&attrs_otel));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Histogram benchmarks
// ---------------------------------------------------------------------------

fn bench_histogram_3_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_histogram_3_attrs");

    let attrs_ro11y: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ];

    // ro11y
    let r_reg = ro11y_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_ro11y);

    group.bench_function("ro11y", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_ro11y));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_hist = o_meter.f64_histogram("request_duration").build();
    o_hist.record(42.5, &attrs_otel);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_hist.record(black_box(42.5), black_box(&attrs_otel));
        });
    });

    group.finish();
}

fn bench_histogram_5_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_histogram_5_attrs");

    let attrs_ro11y: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
    ];

    // ro11y
    let r_reg = ro11y_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_ro11y);

    group.bench_function("ro11y", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_ro11y));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_hist = o_meter.f64_histogram("request_duration").build();
    o_hist.record(42.5, &attrs_otel);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_hist.record(black_box(42.5), black_box(&attrs_otel));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// No-attrs baseline (cheapest possible call)
// ---------------------------------------------------------------------------

fn bench_counter_no_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_no_attrs");

    // ro11y
    let r_reg = ro11y_registry();
    let r_ctr = r_reg.counter("simple", "simple counter");
    r_ctr.add(1, &[]);

    group.bench_function("ro11y", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(&[]));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_ctr = o_meter.u64_counter("simple").build();
    o_ctr.add(1, &[]);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_ctr.add(black_box(1), black_box(&[]));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_counter_no_attrs,
    bench_counter_3_attrs,
    bench_counter_5_attrs,
    bench_histogram_3_attrs,
    bench_histogram_5_attrs,
);
criterion_main!(benches);

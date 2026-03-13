//! Head-to-head benchmark: rolly vs opentelemetry_sdk 0.31
//!
//! Compares identical metric operations on the same hardware to produce
//! an apples-to-apples comparison.
//!
//! Run: `cargo bench --features _bench -- comparison`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ---------------------------------------------------------------------------
// rolly setup
// ---------------------------------------------------------------------------
use rolly::bench::*;

fn rolly_registry() -> MetricsRegistry {
    MetricsRegistry::new()
}

// ---------------------------------------------------------------------------
// OpenTelemetry SDK setup
// ---------------------------------------------------------------------------
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::{metrics::Meter, KeyValue};
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

    // rolly
    let r_reg = rolly_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    // warm up the attribute set
    r_ctr.add(
        1,
        &[
            ("method", "GET"),
            ("status", "200"),
            ("region", "us-east-1"),
        ],
    );

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_ctr.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("status", "200"),
                    ("region", "us-east-1"),
                ]),
            );
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_ctr = o_meter.u64_counter("requests").build();
    // warm up
    o_ctr.add(
        1,
        &[
            KeyValue::new("method", "GET"),
            KeyValue::new("status", "200"),
            KeyValue::new("region", "us-east-1"),
        ],
    );

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

    let attrs_rolly: &[(&str, &str)] = &[
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

    // rolly
    let r_reg = rolly_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    r_ctr.add(1, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(attrs_rolly));
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

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ];

    // rolly
    let r_reg = rolly_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_rolly));
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

    let attrs_rolly: &[(&str, &str)] = &[
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

    // rolly
    let r_reg = rolly_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_rolly));
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
// Cold-path benchmarks (first insert — owned_attrs allocation)
// ---------------------------------------------------------------------------

fn bench_counter_3_attrs_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_3_attrs_cold");

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ];

    // rolly — no warmup, each iteration uses a fresh registry
    group.bench_function("rolly", |b| {
        b.iter(|| {
            let reg = rolly_registry();
            let ctr = reg.counter("requests", "total requests");
            ctr.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("status", "200"),
                    ("region", "us-east-1"),
                ]),
            );
        });
    });

    // OTel SDK — no warmup, each iteration uses a fresh provider
    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            let provider = otel_provider();
            let meter = otel_meter(&provider);
            let ctr = meter.u64_counter("requests").build();
            ctr.add(black_box(1), black_box(&attrs_otel));
        });
    });

    group.finish();
}

fn bench_counter_5_attrs_cold(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_5_attrs_cold");

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
    ];

    group.bench_function("rolly", |b| {
        b.iter(|| {
            let reg = rolly_registry();
            let ctr = reg.counter("requests", "total requests");
            ctr.add(
                black_box(1),
                black_box(&[
                    ("method", "GET"),
                    ("status", "200"),
                    ("region", "us-east-1"),
                    ("host", "api.example.com"),
                    ("path", "/api/v1/users"),
                ]),
            );
        });
    });

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            let provider = otel_provider();
            let meter = otel_meter(&provider);
            let ctr = meter.u64_counter("requests").build();
            ctr.add(black_box(1), black_box(&attrs_otel));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Gauge benchmarks
// ---------------------------------------------------------------------------

fn bench_gauge_3_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_gauge_3_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
    ];

    // rolly
    let r_reg = rolly_registry();
    let r_gauge = r_reg.gauge("connections", "active connections");
    r_gauge.set(1.0, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_gauge.set(black_box(42.0), black_box(attrs_rolly));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_gauge = o_meter.f64_gauge("connections").build();
    o_gauge.record(1.0, &attrs_otel);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_gauge.record(black_box(42.0), black_box(&attrs_otel));
        });
    });

    group.finish();
}

fn bench_gauge_5_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_gauge_5_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
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

    // rolly
    let r_reg = rolly_registry();
    let r_gauge = r_reg.gauge("connections", "active connections");
    r_gauge.set(1.0, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_gauge.set(black_box(42.0), black_box(attrs_rolly));
        });
    });

    // OTel SDK
    let o_provider = otel_provider();
    let o_meter = otel_meter(&o_provider);
    let o_gauge = o_meter.f64_gauge("connections").build();
    o_gauge.record(1.0, &attrs_otel);

    group.bench_function("otel_sdk", |b| {
        b.iter(|| {
            o_gauge.record(black_box(42.0), black_box(&attrs_otel));
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 8-attribute benchmarks (typical microservice)
// ---------------------------------------------------------------------------

fn bench_counter_8_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_8_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
    ];

    // rolly
    let r_reg = rolly_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    r_ctr.add(1, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(attrs_rolly));
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

fn bench_histogram_8_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_histogram_8_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
    ];

    let r_reg = rolly_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_rolly));
        });
    });

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
// 10-attribute benchmarks (instrumented service with infra labels)
// ---------------------------------------------------------------------------

fn bench_counter_10_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_10_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
        ("cluster", "main-cluster"),
        ("availability_zone", "us-east-1a"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
        KeyValue::new("cluster", "main-cluster"),
        KeyValue::new("availability_zone", "us-east-1a"),
    ];

    let r_reg = rolly_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    r_ctr.add(1, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(attrs_rolly));
        });
    });

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

fn bench_histogram_10_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_histogram_10_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
        ("cluster", "main-cluster"),
        ("availability_zone", "us-east-1a"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
        KeyValue::new("cluster", "main-cluster"),
        KeyValue::new("availability_zone", "us-east-1a"),
    ];

    let r_reg = rolly_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_rolly));
        });
    });

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
// 16-attribute benchmarks (fully instrumented production with k8s labels)
// ---------------------------------------------------------------------------

fn bench_counter_16_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_counter_16_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
        ("cluster", "main-cluster"),
        ("availability_zone", "us-east-1a"),
        ("namespace", "default"),
        ("deployment", "user-service-v1"),
        ("pod_name", "user-service-v1-7b9f4"),
        ("container_id", "a1b2c3d4e5f6"),
        ("replica", "2"),
        ("instance_type", "m5.xlarge"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
        KeyValue::new("cluster", "main-cluster"),
        KeyValue::new("availability_zone", "us-east-1a"),
        KeyValue::new("namespace", "default"),
        KeyValue::new("deployment", "user-service-v1"),
        KeyValue::new("pod_name", "user-service-v1-7b9f4"),
        KeyValue::new("container_id", "a1b2c3d4e5f6"),
        KeyValue::new("replica", "2"),
        KeyValue::new("instance_type", "m5.xlarge"),
    ];

    let r_reg = rolly_registry();
    let r_ctr = r_reg.counter("requests", "total requests");
    r_ctr.add(1, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_ctr.add(black_box(1), black_box(attrs_rolly));
        });
    });

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

fn bench_histogram_16_attrs(c: &mut Criterion) {
    let mut group = c.benchmark_group("comparison_histogram_16_attrs");

    let attrs_rolly: &[(&str, &str)] = &[
        ("method", "GET"),
        ("status", "200"),
        ("region", "us-east-1"),
        ("host", "api.example.com"),
        ("path", "/api/v1/users"),
        ("service", "user-service"),
        ("version", "1.4.2"),
        ("environment", "production"),
        ("cluster", "main-cluster"),
        ("availability_zone", "us-east-1a"),
        ("namespace", "default"),
        ("deployment", "user-service-v1"),
        ("pod_name", "user-service-v1-7b9f4"),
        ("container_id", "a1b2c3d4e5f6"),
        ("replica", "2"),
        ("instance_type", "m5.xlarge"),
    ];

    let attrs_otel = [
        KeyValue::new("method", "GET"),
        KeyValue::new("status", "200"),
        KeyValue::new("region", "us-east-1"),
        KeyValue::new("host", "api.example.com"),
        KeyValue::new("path", "/api/v1/users"),
        KeyValue::new("service", "user-service"),
        KeyValue::new("version", "1.4.2"),
        KeyValue::new("environment", "production"),
        KeyValue::new("cluster", "main-cluster"),
        KeyValue::new("availability_zone", "us-east-1a"),
        KeyValue::new("namespace", "default"),
        KeyValue::new("deployment", "user-service-v1"),
        KeyValue::new("pod_name", "user-service-v1-7b9f4"),
        KeyValue::new("container_id", "a1b2c3d4e5f6"),
        KeyValue::new("replica", "2"),
        KeyValue::new("instance_type", "m5.xlarge"),
    ];

    let r_reg = rolly_registry();
    let r_hist = r_reg.histogram(
        "request_duration",
        "HTTP request duration",
        &[5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0],
    );
    r_hist.observe(42.5, attrs_rolly);

    group.bench_function("rolly", |b| {
        b.iter(|| {
            r_hist.observe(black_box(42.5), black_box(attrs_rolly));
        });
    });

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

criterion_group!(
    benches,
    bench_counter_3_attrs,
    bench_counter_5_attrs,
    bench_counter_8_attrs,
    bench_counter_10_attrs,
    bench_counter_16_attrs,
    bench_counter_3_attrs_cold,
    bench_counter_5_attrs_cold,
    bench_gauge_3_attrs,
    bench_gauge_5_attrs,
    bench_histogram_3_attrs,
    bench_histogram_5_attrs,
    bench_histogram_8_attrs,
    bench_histogram_10_attrs,
    bench_histogram_16_attrs,
);
criterion_main!(benches);

# ro11y

Lightweight Rust observability. Hand-rolled OTLP protobuf over HTTP, built on [tracing](https://docs.rs/tracing).

## Core and middleware

ro11y has two layers:

**Generic core** — works with any Rust application, not just HTTP servers:
- Custom `tracing::Layer` that captures all spans and events
- Encodes them as OTLP protobuf (`ExportTraceServiceRequest`, `ExportLogsServiceRequest`)
- Ships via HTTP POST to any OTLP-compatible collector (Vector, Grafana Alloy, OTEL Collector)
- Dual output: OTLP HTTP primary + JSON stderr fallback (local dev / CloudWatch)
- Background exporter with batching (512 items / 1s window), concurrent workers, and 3-retry exponential backoff — telemetry never blocks your application
- Native OTLP metrics with Counter and Gauge instruments, client-side aggregation, and `ExportMetricsServiceRequest` export
- Process metrics (CPU, memory) via `/proc` polling on Linux

**HTTP middleware** (optional, `tower` feature) — framework-specific request instrumentation:
- Tower middleware for Axum
- Extracts request IDs (CloudFront, `x-request-id`, or any header), generates deterministic trace IDs via BLAKE3
- Creates request spans with method, path, status, latency
- Emits RED metrics (request duration, count, errors)
- W3C `traceparent` propagation for outbound requests

Any `tracing` span or event from anywhere in your application — HTTP handlers, background tasks, queue consumers, batch jobs — flows through the same OTLP export pipeline.

## Signals

| Signal  | Format                         | Standard |
|---------|--------------------------------|----------|
| Traces  | OTLP `ExportTraceServiceRequest` protobuf | Yes |
| Logs    | OTLP `ExportLogsServiceRequest` protobuf  | Yes |
| Metrics | OTLP `ExportMetricsServiceRequest` protobuf | Yes |

All three signals follow the [OTLP specification](https://opentelemetry.io/docs/specs/otlp/) and are encoded as native protobuf. Any OTLP-compatible backend can ingest them directly.

### Metrics

ro11y provides Counter and Gauge instruments with client-side aggregation. Metrics are accumulated in-process and flushed as `ExportMetricsServiceRequest` on a configurable interval (default 10s).

```rust
use ro11y::{counter, gauge};

// Counters are monotonic and cumulative
let req_counter = counter("http.server.requests", "Total HTTP requests");
req_counter.add(1, &[("method", "GET"), ("status", "200")]);

// Gauges record last-value
let mem_gauge = gauge("process.memory.usage", "Memory usage in bytes");
mem_gauge.set(1_048_576.0, &[("unit", "bytes")]);
```

Attribute order does not matter — `[("a", "1"), ("b", "2")]` and `[("b", "2"), ("a", "1")]` aggregate to the same data point.

## Usage

```rust
use ro11y::{init, TelemetryConfig};
use std::time::Duration;

let _guard = init(TelemetryConfig {
    service_name: "my-service",
    service_version: env!("CARGO_PKG_VERSION"),
    environment: "prod",
    otlp_traces_endpoint: Some("http://vector:4318"),
    otlp_logs_endpoint: Some("http://vector:4318"),
    otlp_metrics_endpoint: Some("http://vector:4318"),
    log_to_stderr: true,
    use_metrics_interval: Some(Duration::from_secs(30)),
    metrics_flush_interval: None, // default 10s
});

// All tracing spans/events are now exported as OTLP protobuf
tracing::info_span!("process_job", job_id = 42).in_scope(|| {
    tracing::info!("job completed");
});
```

Endpoints can be configured independently — send traces to Jaeger, logs to Vector, and metrics to a different collector:

```rust
let _guard = init(TelemetryConfig {
    service_name: "my-service",
    service_version: env!("CARGO_PKG_VERSION"),
    environment: "prod",
    otlp_traces_endpoint: Some("http://jaeger:4318"),
    otlp_logs_endpoint: Some("http://vector:4318"),
    otlp_metrics_endpoint: Some("http://prometheus-gateway:4318"),
    log_to_stderr: false,
    use_metrics_interval: None,
    metrics_flush_interval: Some(Duration::from_secs(15)),
});
```

Set any endpoint to `None` to disable that signal.

### HTTP middleware (Axum/Tower)

The `tower` feature is enabled by default.

```rust
let app = axum::Router::new()
    .route("/health", axum::routing::get(health))
    .layer(ro11y::request_layer())       // inbound: request spans + RED metrics
    .layer(ro11y::propagation_layer());  // outbound: W3C traceparent injection
```

To disable Tower middleware (e.g. for non-HTTP applications):

```toml
[dependencies]
ro11y = { version = "0.3", default-features = false }
```

## Pipeline

```
Application (tracing) → ro11y (protobuf) → HTTP POST → Vector/Collector (OTLP) → storage
```

## Why not OpenTelemetry SDK?

- Version lock-step across `opentelemetry-*` crates
- ~120 transitive dependencies, 3+ minute compile times
- Shutdown footgun (`drop()` doesn't flush)
- gRPC bloat from `tonic`/`prost`

ro11y hand-rolls the protobuf wire format (~200 lines). The format has been stable since 2008.

## Dependencies

7 direct dependencies. No `opentelemetry`, `tonic`, or `prost`.

## License

MIT OR Apache-2.0

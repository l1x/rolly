# ro11y

Lightweight Rust observability. Hand-rolled OTLP protobuf over HTTP, built on [tracing](https://docs.rs/tracing).

## What it does

- Exports traces and logs as native OTLP protobuf over HTTP — no gRPC, no `tonic`, no `prost`
- Dual output: OTLP HTTP to [Vector](https://vector.dev) + JSON stderr (local dev / CloudWatch fallback)
- Deterministic trace IDs from CloudFront request IDs via BLAKE3
- RED metrics (request duration, count, errors) as structured log events
- W3C `traceparent` propagation for outbound requests
- Tower middleware for Axum integration
- Fire-and-forget with 3-retry exponential backoff — telemetry never blocks your service
- 7 direct dependencies, ~2000 lines of code

## Usage

```rust
use ro11y::{init, TelemetryConfig};
use std::time::Duration;

let _guard = init(TelemetryConfig {
    service_name: "my-service",
    service_version: env!("CARGO_PKG_VERSION"),
    environment: "prod",
    otlp_endpoint: Some("http://vector:4318"),
    use_metrics_interval: Some(Duration::from_secs(30)),
});

// Tower middleware for Axum
let app = axum::Router::new()
    .route("/health", axum::routing::get(health))
    .layer(ro11y::request_layer())       // inbound: request spans + RED metrics
    .layer(ro11y::propagation_layer());  // outbound: W3C traceparent injection
```

## Pipeline

```
Service (tracing) → ro11y (protobuf) → HTTP POST → Vector (OTLP source) → S3 Parquet
```

## Why not OpenTelemetry SDK?

- Version lock-step across `opentelemetry-*` crates
- ~120 transitive dependencies, 3+ minute compile times
- Shutdown footgun (`drop()` doesn't flush)
- gRPC bloat from `tonic`/`prost`

ro11y hand-rolls the protobuf wire format (~200 lines). The format has been stable since 2008.

## License

MIT OR Apache-2.0

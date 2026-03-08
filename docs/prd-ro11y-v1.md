# PRD: ro11y — Lightweight Rust Observability

**Version:** 1.0
**Date:** 2026-03-07
**Status:** Active
**Target Scale:** 2 Billion+ HTTP requests / month (~770+ req/s sustained)
**Repository:** [github.com/l1x/ro11y](https://github.com/l1x/ro11y)
**crates.io:** [crates.io/crates/ro11y](https://crates.io/crates/ro11y)
**License:** MIT OR Apache-2.0

---

## 1. Problem Statement

The OpenTelemetry Rust SDK is the standard approach for exporting telemetry from Rust services. In practice, it introduces significant pain:

- **Version lock-step** — all `opentelemetry-*` crates must version-match exactly. One patch bump breaks the others.
- **Dependency bloat** — ~120 transitive dependencies. gRPC export via `tonic`/`prost` adds 30+ seconds to compile times.
- **Shutdown footgun** — `TracerProvider::drop()` does not flush. Must call `shutdown()` explicitly before the tokio runtime drops, or lose tail spans.
- **Fragile bridge** — `tracing-opentelemetry` breaks on every OTEL release. Undocumented version matrix between `tracing-subscriber`, `opentelemetry`, and `opentelemetry_sdk`.
- **Extreme Scale** — High-throughput services (2B+ req/month) struggle with the overhead of standard SDKs and require surgical control over allocations and export batching.

ro11y replaces the OpenTelemetry SDK entirely with ~2,000 lines of hand-rolled code and 7 direct dependencies.

## 2. What ro11y Is

A Rust crate that bridges the `tracing` ecosystem to OTLP-compatible collectors via hand-rolled protobuf over HTTP. It is:

- **An OTLP exporter** — encodes traces and logs as native `ExportTraceServiceRequest` and `ExportLogsServiceRequest` protobuf, POSTs to any OTLP HTTP endpoint.
- **A tracing subscriber layer** — plugs into `tracing-subscriber` registry alongside `fmt::Layer` and `EnvFilter`. No new instrumentation API.
- **HTTP middleware** — Tower layers for inbound request instrumentation and outbound trace context propagation.
- **Framework-agnostic at the core** — the export pipeline works with any `tracing` span or event, not just HTTP. Queue consumers, batch jobs, CLI tools — anything that uses `tracing` gets OTLP export for free.

## 3. What ro11y Is Not

- Not a dashboarding or alerting system
- Not a replacement for `tracing` — it builds on top of it
- Not a distributed tracing system — it exports to one (via Vector, OTEL Collector, etc.)

## 4. Current Implementation (v0.1.x)

### 4.1 Public API

```rust
// Initialize the full telemetry stack
pub fn init(config: TelemetryConfig) -> TelemetryGuard

pub struct TelemetryConfig {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub environment: &'static str,
    /// OTLP HTTP endpoint for traces. If None, trace export is disabled.
    pub otlp_traces_endpoint: Option<&'static str>,
    /// OTLP HTTP endpoint for logs. Can differ from traces endpoint for scaling.
    pub otlp_logs_endpoint: Option<&'static str>,
    pub log_to_stderr: bool,                         // JSON stderr fallback
    pub use_metrics_interval: Option<Duration>,      // Linux /proc polling
    pub sampling_rate: f64,                          // 1.0 = 100%, 0.01 = 1%
}

pub struct TelemetryGuard { .. }  // Flushes on drop
```

### 4.2 Architecture

```
init() sets up:
├── EnvFilter (RUST_LOG, default: info,tower_http=info)
├── fmt::Layer (JSON to stderr — CloudWatch fallback, always on)
└── OtlpLayer (custom tracing::Layer)
    ├── on_new_span() → assign trace_id, span_id, store in extensions
    ├── on_event() → encode as ExportLogsServiceRequest → POST /v1/logs
    └── on_close() → encode as ExportTraceServiceRequest → POST /v1/traces
        └── Exporter (background tokio task)
            ├── Batching Buffer: collects up to 512 items or 1s window
            ├── Concurrent Workers: Parallel HTTP POSTs to avoid HoL blocking
            ├── reqwest HTTP POST with Content-Type: application/x-protobuf
            ├── Exponential backoff: 100ms → 400ms → 1600ms
            └── Drop Strategy: "Drop-Newest" if buffer full; log `telemetry_dropped_total`
```

### 4.4 Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `blake3` | 1 | Hash CloudFront ID → deterministic trace_id |
| `bytes` | 1 | Efficient buffer operations (O(1) clone via Arc) |
| `http` | 1 | HTTP types for Tower middleware |
| `pin-project-lite` | 0.2 | Future pinning for Tower service |
| `rand` | 0.9 | Random span IDs |
| `reqwest` | 0.12 | HTTP POST to OTLP endpoint (rustls-tls) |
| `tokio` | 1 | Async runtime (sync channels, timers) |
| `tower` | 0.5 | Middleware traits (Layer, Service) |
| `tracing` | 0.1 | Instrumentation API |
| `tracing-subscriber` | 0.3 | Subscriber registry, JSON formatter, env filter |
| `uuid` | 1 | UUID v4 for trace IDs (when no CloudFront ID) |
| `libc` | 0.2 | Linux only: `sysconf()` for page size and clock ticks |

### 4.7 Trace ID Strategy

| Source | Input | trace_id |
|--------|-------|----------|
| CloudFront → ALB → ECS | `x-amz-cf-id` header | `BLAKE3(request_id)[0..16]` — deterministic |
| Direct API call (no CF) | None | UUID v4 — random |

### 4.8 HTTP Middleware

**`CfRequestIdLayer`** (inbound requests):
1. Extract `x-amz-cf-id` header
2. Generate trace_id and span_id
3. **PII Protection:** Scrub query parameters from `http.uri` before recording.
4. Create `info_span!("request", ...)`
5. On response: record status code, latency, emit RED metric events

### 4.10 Non-Functional Requirements (High Scale)

| NFR | Implementation |
|-----|---------------|
| **Zero-Block Policy** | Telemetry must not block the service; fire-and-forget via tokio channel. |
| **Batching** | Telemetry MUST be batched (512 items or 1s) to prevent network saturation. |
| **PII Scrubbing** | Middleware MUST strip sensitive data from URLs before telemetry capture. |
| **Sampling** | Probabilistic sampling for traces to manage storage costs at 2B req/month. |
| **Backpressure** | When internal buffer (1024) is full, drop newest telemetry and emit `telemetry_dropped_total`. |
| **No Recursion** | Exporter uses `eprintln!()` to avoid infinite loops with `tracing`. |

## 5. Cost at Scale (2B requests/month)

| Component | Sizing | Monthly Cost |
|-----------|--------|-------------|
| Vector ECS | 2 tasks, 2 vCPU + 4 GB each | ~$145 |
| S3 Storage | ~565 GB average (30d traces/logs, 90d metrics) | ~$13 |
| **Total** | | **~$304/month** |

*Note: Datadog for the same workload would cost $6,700–$20,000+/month.*

## 8. Metrics Gap

At 2B requests/month, the "Log-based metrics" workaround generates 4B+ events, causing 1,500x more noise than native OTLP metrics. Native metrics with client-side aggregation is a **P0 requirement for v1.0**.

Implementing `ExportMetricsServiceRequest` requires:
1. **Protobuf encoder** for `ResourceMetrics` and `NumberDataPoint`.
2. **Client-side aggregation** — accumulate values over 10s intervals before flush.

## 10. Milestones

| Version | Scope |
|---------|-------|
| **v0.1.x** | Core OTLP traces + logs, Tower middleware, hand-rolled protobuf. |
| **v0.2.0** | **Batching & Concurrency:** Buffer items before export; parallel HTTP workers. **PII Scrubbing.** |
| **v0.3.0** | **Native OTLP Metrics:** `ExportMetricsServiceRequest`, Counter + Gauge instruments. |
| **v0.4.0** | **Trace Sampling:** Deterministic head-based probabilistic sampling via trace_id. |
| **v1.0.0** | **Stability:** Stable API, battle-tested at 2B+ req/month. |

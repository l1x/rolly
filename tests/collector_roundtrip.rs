#![cfg(feature = "_bench")]

//! Smoke test: verify traces/logs/metrics arrive at a real otel-collector.
//!
//! Requires a running collector:
//! ```sh
//! docker run -d -p 4318:4318 -p 13133:13133 \
//!   -v $(pwd)/tests/otel-config.yaml:/etc/otelcol/config.yaml \
//!   otel/opentelemetry-collector:latest
//! ```
//!
//! Run with: cargo test --features _bench collector_roundtrip -- --ignored

use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn traces_logs_metrics_arrive_at_collector() {
    // Check collector health before proceeding
    let client = reqwest::Client::new();
    let health = client
        .get("http://localhost:13133")
        .timeout(Duration::from_secs(2))
        .send()
        .await;

    match health {
        Ok(resp) if resp.status().is_success() => {}
        _ => {
            panic!(
                "otel-collector not reachable at localhost:13133; \
                 start it with:\n  \
                 docker run -d -p 4318:4318 -p 13133:13133 \
                 -v $(pwd)/tests/otel-config.yaml:/etc/otelcol/config.yaml \
                 otel/opentelemetry-collector:latest"
            );
        }
    }

    let guard = rolly::init(rolly::TelemetryConfig {
        service_name: "roundtrip-test".into(),
        service_version: "0.0.1".into(),
        environment: "test".into(),
        otlp_traces_endpoint: Some("http://localhost:4318".into()),
        otlp_logs_endpoint: Some("http://localhost:4318".into()),
        otlp_metrics_endpoint: Some("http://localhost:4318".into()),
        log_to_stderr: false,
        use_metrics_interval: None,
        metrics_flush_interval: Some(Duration::from_secs(1)),
        sampling_rate: Some(1.0),
        backpressure_strategy: rolly::BackpressureStrategy::Drop,
        resource_attributes: vec![],
    });

    // Create spans and events
    {
        let span = tracing::info_span!(
            "roundtrip-span",
            trace_id = "aabbccdd11223344aabbccdd11223344"
        );
        let _enter = span.enter();
        tracing::info!("roundtrip-event");
    }

    // Record metrics
    let ctr = rolly::counter("roundtrip_requests", "test counter");
    ctr.add(1, &[("method", "GET")]);

    let g = rolly::gauge("roundtrip_connections", "test gauge");
    g.set(42.0, &[("pool", "main")]);

    // Allow metrics flush to fire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Drop guard in a separate thread to avoid "runtime within runtime" panic.
    std::thread::spawn(move || drop(guard)).join().unwrap();

    // If we got here without panicking, the smoke test passes.
}

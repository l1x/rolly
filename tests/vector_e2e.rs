#![cfg(feature = "_bench")]

//! End-to-end test using Vector as OTLP collector.
//!
//! Vector receives OTLP traces/logs/metrics over HTTP and writes them
//! as newline-delimited JSON to files. The test reads those files and
//! asserts that expected data arrived correctly parsed.
//!
//! # Setup
//!
//! ```sh
//! docker compose -f docker-compose.e2e.yaml up -d
//! ```
//!
//! # Run
//!
//! ```sh
//! cargo test --features _bench --release --test vector_e2e -- --ignored --nocapture
//! ```
//!
//! # Cleanup
//!
//! ```sh
//! docker compose -f docker-compose.e2e.yaml down -v
//! ```

use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// Read an NDJSON file, retrying until it contains at least one parseable line
/// or the timeout expires.
fn read_ndjson_with_retry(path: &Path, timeout: Duration) -> Vec<Value> {
    let start = Instant::now();
    loop {
        if let Ok(contents) = std::fs::read_to_string(path) {
            let lines: Vec<Value> = contents
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            if !lines.is_empty() {
                return lines;
            }
        }
        if start.elapsed() > timeout {
            panic!(
                "Timed out waiting for data in {}: file {} after {:?}",
                path.display(),
                if path.exists() {
                    "exists but is empty"
                } else {
                    "does not exist"
                },
                timeout,
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Recursively search a JSON value for a string containing `needle`.
fn json_contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.contains(needle),
        Value::Array(arr) => arr.iter().any(|v| json_contains_string(v, needle)),
        Value::Object(map) => {
            map.keys().any(|k| k.contains(needle))
                || map.values().any(|v| json_contains_string(v, needle))
        }
        _ => false,
    }
}

/// Check if any JSON event in the list contains the given string anywhere
/// in its keys or string values.
fn any_event_contains(events: &[Value], needle: &str) -> bool {
    events.iter().any(|v| json_contains_string(v, needle))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn vector_e2e_traces_logs_metrics() {
    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/vector-output");
    let timeout = Duration::from_secs(10);

    // Ensure output directory exists for the bind mount
    std::fs::create_dir_all(&output_dir).expect("failed to create output directory");

    // ── Health-check Vector ──────────────────────────────────────────
    let client = reqwest::Client::new();
    let health = client
        .post("http://localhost:4318/v1/traces")
        .header("content-type", "application/x-protobuf")
        .body(Vec::new())
        .timeout(Duration::from_secs(2))
        .send()
        .await;

    match health {
        Ok(_) => {} // Any response means Vector is listening
        Err(e) => {
            panic!(
                "Vector not reachable at localhost:4318: {e}\n\
                 Start it with: docker compose -f docker-compose.e2e.yaml up -d"
            );
        }
    }

    // Clean output files from previous runs
    let _ = std::fs::remove_file(output_dir.join("traces.json"));
    let _ = std::fs::remove_file(output_dir.join("logs.json"));
    let _ = std::fs::remove_file(output_dir.join("metrics.json"));
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ── Initialize telemetry ─────────────────────────────────────────
    let guard = rolly::init(rolly::TelemetryConfig {
        service_name: "vector-e2e-test".into(),
        service_version: "0.1.0".into(),
        environment: "e2e".into(),
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

    // ── Send traces ──────────────────────────────────────────────────
    let trace_id_hex = "aabbccdd11223344aabbccdd11223344";
    {
        let span = tracing::info_span!(
            "vector-e2e-span",
            trace_id = trace_id_hex,
            test.attr = "e2e-attr-value",
        );
        let _enter = span.enter();

        // ── Send logs (inside span for trace_id correlation) ─────────
        tracing::info!("e2e-log-message");
    }

    // ── Send metrics ─────────────────────────────────────────────────
    let ctr = rolly::counter("e2e_requests_total", "e2e test counter");
    ctr.add(1, &[("method", "GET")]);

    let g = rolly::gauge("e2e_active_connections", "e2e test gauge");
    g.set(42.0, &[("pool", "main")]);

    // Wait for the metrics aggregation loop to flush (interval = 1s)
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Flush and shutdown exporter
    std::thread::spawn(move || drop(guard)).join().unwrap();

    // Give Vector a moment to write files to disk
    tokio::time::sleep(Duration::from_secs(1)).await;

    // ── Assert traces ────────────────────────────────────────────────
    let traces = read_ndjson_with_retry(&output_dir.join("traces.json"), timeout);
    assert!(
        any_event_contains(&traces, "vector-e2e-span"),
        "traces should contain span name 'vector-e2e-span'"
    );
    assert!(
        any_event_contains(&traces, "vector-e2e-test"),
        "traces should contain service name 'vector-e2e-test'"
    );
    assert!(
        any_event_contains(&traces, "e2e-attr-value"),
        "traces should contain attribute value 'e2e-attr-value'"
    );

    // ── Assert logs ──────────────────────────────────────────────────
    let logs = read_ndjson_with_retry(&output_dir.join("logs.json"), timeout);
    assert!(
        any_event_contains(&logs, "e2e-log-message"),
        "logs should contain message 'e2e-log-message'"
    );

    // ── Assert metrics ───────────────────────────────────────────────
    let metrics = read_ndjson_with_retry(&output_dir.join("metrics.json"), timeout);
    assert!(
        any_event_contains(&metrics, "e2e_requests_total"),
        "metrics should contain counter 'e2e_requests_total'"
    );
    assert!(
        any_event_contains(&metrics, "e2e_active_connections"),
        "metrics should contain gauge 'e2e_active_connections'"
    );

    println!("--- Vector E2E test passed ---");
    println!("  Traces:  {} events", traces.len());
    println!("  Logs:    {} events", logs.len());
    println!("  Metrics: {} events", metrics.len());
}

#![cfg(feature = "_bench")]

//! Shutdown and flush reliability tests.
//! Uses Exporter::start() (full HTTP loop) with TCP listeners,
//! and tracing::subscriber::set_default() for thread-local subscribers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rolly::bench::{BackpressureStrategy, Exporter, ExporterConfig, OtlpLayer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;

/// Spawn a TCP server that accepts connections, reads the full HTTP request body,
/// stores it, and responds with the given status line.
fn spawn_http_server(
    listener: TcpListener,
    status: &'static str,
    bodies: Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>,
) {
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let bodies = bodies.clone();
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(65536);
                let mut tmp = [0u8; 8192];
                loop {
                    let n = stream.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&buf[..hdr_end]);
                        let cl: usize = headers
                            .lines()
                            .find_map(|l| {
                                if l.to_lowercase().starts_with("content-length:") {
                                    l.split(':').nth(1)?.trim().parse().ok()
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(0);
                        let body_start = hdr_end + 4;
                        while buf.len() - body_start < cl {
                            let n = stream.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        let body = buf[body_start..body_start + cl].to_vec();
                        if !body.is_empty() {
                            bodies.lock().await.push(body);
                        }
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    status
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn all_spans_arrive_after_flush_and_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bodies: Arc<tokio::sync::Mutex<Vec<Vec<u8>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    spawn_http_server(listener, "200 OK", bodies.clone());

    let exporter = Exporter::start(ExporterConfig {
        traces_url: Some(format!("http://{}/v1/traces", addr)),
        logs_url: None,
        metrics_url: None,
        channel_capacity: 1024,
        batch_size: 512,
        flush_interval: Duration::from_secs(60),
        max_concurrent_exports: 4,
        backpressure_strategy: BackpressureStrategy::Drop,
    });

    let layer = OtlpLayer::new(
        exporter.clone(),
        "flush-test",
        "0.0.1",
        "test",
        true,
        false,
        1.0,
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    for _ in 0..50 {
        let span = tracing::info_span!("flush-span");
        let _enter = span.enter();
    }

    exporter.flush().await;
    exporter.shutdown().await;

    let received = bodies.lock().await;
    let all_bytes: Vec<u8> = received.iter().flatten().copied().collect();
    let count = all_bytes
        .windows(10)
        .filter(|w| *w == b"flush-span")
        .count();
    assert!(
        count >= 50,
        "expected at least 50 'flush-span' in HTTP bodies, got {}",
        count
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_completes_when_endpoint_is_failing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bodies: Arc<tokio::sync::Mutex<Vec<Vec<u8>>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    // Server returning 500 triggers retries (100ms + 400ms + 1600ms = ~2.1s per batch)
    spawn_http_server(listener, "500 Internal Server Error", bodies);

    let exporter = Exporter::start(ExporterConfig {
        traces_url: Some(format!("http://{}/v1/traces", addr)),
        logs_url: None,
        metrics_url: None,
        channel_capacity: 1024,
        batch_size: 512,
        flush_interval: Duration::from_secs(60),
        max_concurrent_exports: 4,
        backpressure_strategy: BackpressureStrategy::Drop,
    });

    let layer = OtlpLayer::new(
        exporter.clone(),
        "fail-test",
        "0.0.1",
        "test",
        true,
        false,
        1.0,
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    {
        let span = tracing::info_span!("will-fail");
        let _enter = span.enter();
    }

    // Flush + shutdown should complete within 10s even with retries
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        exporter.flush().await;
        exporter.shutdown().await;
    })
    .await;

    assert!(
        result.is_ok(),
        "flush+shutdown did not complete within 10s timeout"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flush_waits_for_in_flight_exports() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received_count = Arc::new(AtomicUsize::new(0));
    let count_clone = received_count.clone();

    // Server with 100ms delay before responding 200
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let count = count_clone.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 65536];
                let _ = stream.read(&mut buf).await;
                // Count the request as received immediately
                count.fetch_add(1, Ordering::SeqCst);
                // Delay before responding to simulate slow backend
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
            });
        }
    });

    let exporter = Exporter::start(ExporterConfig {
        traces_url: Some(format!("http://{}/v1/traces", addr)),
        logs_url: None,
        metrics_url: None,
        channel_capacity: 1024,
        batch_size: 1, // flush on every message
        flush_interval: Duration::from_secs(60),
        max_concurrent_exports: 8,
        backpressure_strategy: BackpressureStrategy::Drop,
    });

    let layer = OtlpLayer::new(
        exporter.clone(),
        "inflight-test",
        "0.0.1",
        "test",
        true,
        false,
        1.0,
    );
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    for _ in 0..5 {
        let span = tracing::info_span!("inflight-span");
        let _enter = span.enter();
    }

    // flush() waits for all in-flight HTTP exports to complete
    exporter.flush().await;

    let count = received_count.load(Ordering::SeqCst);
    assert!(
        count >= 5,
        "expected at least 5 requests received by the time flush returns, got {}",
        count
    );

    exporter.shutdown().await;
}

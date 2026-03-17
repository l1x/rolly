use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Read a full HTTP request from a stream, extract the body, and send 200 OK.
/// Returns the request body bytes.
async fn handle_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];

    loop {
        let n = stream.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);

        if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..hdr_end]);
            let content_length: usize = headers
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
            if buf.len() - body_start >= content_length {
                let body = buf[body_start..body_start + content_length].to_vec();

                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;

                return body;
            }
        }
    }

    Vec::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn init_creates_spans_that_arrive_as_otlp_protobuf() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint: &'static str = Box::leak(format!("http://{}", addr).into_boxed_str());

    let (body_tx, mut body_rx) = mpsc::channel::<Vec<u8>>(32);

    // Spawn HTTP server that captures request bodies
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let body_tx = body_tx.clone();
            tokio::spawn(async move {
                let body = handle_http_request(&mut stream).await;
                if !body.is_empty() {
                    let _ = body_tx.send(body).await;
                }
            });
        }
    });

    // Initialize telemetry pointing at our test server
    let guard = rolly::init(rolly::TelemetryConfig {
        service_name: "e2e-test-service".into(),
        service_version: "0.0.1".into(),
        environment: "test".into(),
        otlp_traces_endpoint: Some(endpoint.to_string()),
        otlp_logs_endpoint: Some(endpoint.to_string()),
        otlp_metrics_endpoint: None,
        log_to_stderr: false,
        use_metrics_interval: None,
        metrics_flush_interval: None,
        sampling_rate: None,
        backpressure_strategy: rolly::BackpressureStrategy::Drop,
    });

    // Create a span with a known trace_id
    let trace_id_hex = "aabbccddaabbccddaabbccddaabbccdd";
    {
        let span = tracing::info_span!("e2e-test-span", trace_id = trace_id_hex);
        let _enter = span.enter();
    }

    // Drop guard in a separate thread to avoid "runtime within runtime" panic.
    // The guard's Drop impl creates a new current_thread runtime for flushing.
    std::thread::spawn(move || drop(guard)).join().unwrap();

    // Collect received bodies with timeout
    let mut bodies = Vec::new();
    while let Ok(Some(body)) = tokio::time::timeout(Duration::from_secs(5), body_rx.recv()).await {
        bodies.push(body);
        // Stop once we've seen the trace span (init log + span trace expected)
        if bodies
            .iter()
            .any(|b| b.windows(13).any(|w| w == b"e2e-test-span"))
        {
            break;
        }
    }

    let trace_id_bytes: [u8; 16] = [
        0xaa, 0xbb, 0xcc, 0xdd, 0xaa, 0xbb, 0xcc, 0xdd, 0xaa, 0xbb, 0xcc, 0xdd, 0xaa, 0xbb, 0xcc,
        0xdd,
    ];

    assert!(
        bodies
            .iter()
            .any(|b| b.windows(16).any(|w| w == trace_id_bytes)),
        "trace_id bytes not found in received bodies"
    );

    assert!(
        bodies
            .iter()
            .any(|b| b.windows(13).any(|w| w == b"e2e-test-span")),
        "span name 'e2e-test-span' not found in received bodies"
    );

    assert!(
        bodies
            .iter()
            .any(|b| b.windows(16).any(|w| w == b"e2e-test-service")),
        "service.name 'e2e-test-service' not found in received bodies"
    );
}

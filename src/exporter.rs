use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;

/// Total number of telemetry messages dropped due to a full channel.
static DROPPED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Return the total number of dropped telemetry messages.
pub(crate) fn dropped_total() -> u64 {
    DROPPED_TOTAL.load(Ordering::Relaxed)
}

/// Reset the drop counter (test-only).
#[cfg(test)]
pub(crate) fn reset_dropped_total() {
    DROPPED_TOTAL.store(0, Ordering::Relaxed);
}

/// Message types sent to the exporter background task.
#[derive(Debug)]
pub enum ExportMessage {
    Traces(Bytes),
    Logs(Bytes),
    Metrics(Bytes),
    Flush(tokio::sync::oneshot::Sender<()>),
    Shutdown,
}

/// Configuration for the exporter.
pub struct ExporterConfig {
    pub traces_url: Option<String>,
    pub logs_url: Option<String>,
    pub metrics_url: Option<String>,
    pub channel_capacity: usize,
    pub batch_size: usize,
    pub flush_interval: Duration,
    pub max_concurrent_exports: usize,
}

/// Handle to the exporter background task.
#[derive(Clone)]
pub struct Exporter {
    tx: mpsc::Sender<ExportMessage>,
}

impl Exporter {
    /// Start the exporter background task. Returns a handle for sending data.
    pub fn start(config: ExporterConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");
        tokio::spawn(exporter_loop(
            rx,
            client,
            config.traces_url,
            config.logs_url,
            config.metrics_url,
            config.batch_size,
            config.flush_interval,
            config.max_concurrent_exports,
        ));
        Self { tx }
    }

    /// Create an exporter for testing that doesn't spawn the HTTP loop.
    /// Returns the exporter and receiver so tests can read messages directly.
    #[cfg(any(test, feature = "_bench"))]
    pub fn start_test() -> (Self, mpsc::Receiver<ExportMessage>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }

    /// Create a test exporter with a specific channel capacity.
    #[cfg(any(test, feature = "_bench"))]
    pub fn start_test_with_capacity(capacity: usize) -> (Self, mpsc::Receiver<ExportMessage>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx }, rx)
    }

    /// Send encoded trace data to the exporter.
    pub fn send_traces(&self, data: Vec<u8>) {
        if self
            .tx
            .try_send(ExportMessage::Traces(Bytes::from(data)))
            .is_err()
        {
            DROPPED_TOTAL.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Send encoded log data to the exporter.
    pub fn send_logs(&self, data: Vec<u8>) {
        if self
            .tx
            .try_send(ExportMessage::Logs(Bytes::from(data)))
            .is_err()
        {
            DROPPED_TOTAL.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Send encoded metrics data to the exporter.
    pub fn send_metrics(&self, data: Vec<u8>) {
        if self
            .tx
            .try_send(ExportMessage::Metrics(Bytes::from(data)))
            .is_err()
        {
            DROPPED_TOTAL.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Flush all pending data. Blocks until the exporter has processed everything.
    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(ExportMessage::Flush(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// Signal the exporter to stop after draining remaining messages.
    pub async fn shutdown(&self) {
        let _ = self.tx.send(ExportMessage::Shutdown).await;
    }
}

const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(400),
    Duration::from_millis(1600),
];

#[allow(clippy::too_many_arguments)]
async fn exporter_loop(
    mut rx: mpsc::Receiver<ExportMessage>,
    client: reqwest::Client,
    traces_url: Option<String>,
    logs_url: Option<String>,
    metrics_url: Option<String>,
    batch_size: usize,
    flush_interval: Duration,
    max_concurrent_exports: usize,
) {
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    use tokio::task::JoinSet;

    let semaphore = Arc::new(Semaphore::new(max_concurrent_exports));
    let mut join_set: JoinSet<()> = JoinSet::new();
    let mut trace_batch: Vec<Bytes> = Vec::new();
    let mut log_batch: Vec<Bytes> = Vec::new();
    let mut metrics_batch: Vec<Bytes> = Vec::new();
    let mut interval = tokio::time::interval(flush_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Consume the first immediate tick
    interval.tick().await;

    loop {
        tokio::select! {
            biased;
            msg = rx.recv() => {
                match msg {
                    Some(ExportMessage::Traces(data)) => {
                        trace_batch.push(data);
                        if trace_batch.len() >= batch_size {
                            flush_batch(
                                &mut trace_batch,
                                traces_url.as_deref(),
                                &client,
                                &semaphore,
                                &mut join_set,
                            );
                        }
                    }
                    Some(ExportMessage::Logs(data)) => {
                        log_batch.push(data);
                        if log_batch.len() >= batch_size {
                            flush_batch(
                                &mut log_batch,
                                logs_url.as_deref(),
                                &client,
                                &semaphore,
                                &mut join_set,
                            );
                        }
                    }
                    Some(ExportMessage::Metrics(data)) => {
                        metrics_batch.push(data);
                        if metrics_batch.len() >= batch_size {
                            flush_batch(
                                &mut metrics_batch,
                                metrics_url.as_deref(),
                                &client,
                                &semaphore,
                                &mut join_set,
                            );
                        }
                    }
                    Some(ExportMessage::Flush(done)) => {
                        flush_batch(
                            &mut trace_batch,
                            traces_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        flush_batch(
                            &mut log_batch,
                            logs_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        flush_batch(
                            &mut metrics_batch,
                            metrics_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        // Wait for all in-flight exports to complete
                        while join_set.join_next().await.is_some() {}
                        let _ = done.send(());
                    }
                    Some(ExportMessage::Shutdown) | None => {
                        flush_batch(
                            &mut trace_batch,
                            traces_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        flush_batch(
                            &mut log_batch,
                            logs_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        flush_batch(
                            &mut metrics_batch,
                            metrics_url.as_deref(),
                            &client,
                            &semaphore,
                            &mut join_set,
                        );
                        while join_set.join_next().await.is_some() {}
                        break;
                    }
                }
            }
            _ = interval.tick() => {
                flush_batch(
                    &mut trace_batch,
                    traces_url.as_deref(),
                    &client,
                    &semaphore,
                    &mut join_set,
                );
                flush_batch(
                    &mut log_batch,
                    logs_url.as_deref(),
                    &client,
                    &semaphore,
                    &mut join_set,
                );
                flush_batch(
                    &mut metrics_batch,
                    metrics_url.as_deref(),
                    &client,
                    &semaphore,
                    &mut join_set,
                );
            }
        }

        // Reap completed tasks without blocking
        while let Some(result) = join_set.try_join_next() {
            drop(result);
        }
    }
}

/// Concatenate batch payloads and spawn a concurrent POST task.
///
/// Protobuf repeated fields merge on concatenation, so multiple
/// `ExportTraceServiceRequest` payloads concatenated produce a valid
/// message with multiple `ResourceSpans`.
fn flush_batch(
    batch: &mut Vec<Bytes>,
    url: Option<&str>,
    client: &reqwest::Client,
    semaphore: &std::sync::Arc<tokio::sync::Semaphore>,
    join_set: &mut tokio::task::JoinSet<()>,
) {
    if batch.is_empty() {
        return;
    }
    let url = match url {
        Some(u) => u.to_string(),
        None => {
            batch.clear();
            return;
        }
    };

    let total_len: usize = batch.iter().map(|b| b.len()).sum();
    let mut payload = Vec::with_capacity(total_len);
    for item in batch.drain(..) {
        payload.extend_from_slice(&item);
    }
    let data = Bytes::from(payload);
    let client = client.clone();
    let semaphore = semaphore.clone();

    join_set.spawn(async move {
        let _permit = semaphore.acquire().await;
        post_with_retry(&client, &url, data).await;
    });
}

/// POST with exponential backoff. On total failure, drop the batch.
///
/// Uses `eprintln!` intentionally — not `tracing::warn!` — because this runs
/// inside the telemetry pipeline. Using tracing here would re-enter the OtlpLayer
/// and cause infinite recursion.
async fn post_with_retry(client: &reqwest::Client, url: &str, data: Bytes) {
    for (attempt, delay) in RETRY_DELAYS.iter().enumerate() {
        match client
            .post(url)
            .header("Content-Type", "application/x-protobuf")
            .body(data.clone()) // Bytes::clone is O(1) — just an Arc bump
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return,
            Ok(resp) => {
                eprintln!(
                    "pz-o11y: export attempt {}/{} to {} failed: HTTP {}",
                    attempt + 1,
                    RETRY_DELAYS.len(),
                    url,
                    resp.status()
                );
            }
            Err(e) => {
                eprintln!(
                    "pz-o11y: export attempt {}/{} to {} failed: {}",
                    attempt + 1,
                    RETRY_DELAYS.len(),
                    url,
                    e
                );
            }
        }
        tokio::time::sleep(*delay).await;
    }
    eprintln!(
        "pz-o11y: dropping batch after {} retries",
        RETRY_DELAYS.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_counter_starts_at_zero() {
        reset_dropped_total();
        assert_eq!(dropped_total(), 0);
    }

    #[tokio::test]
    async fn drop_counter_increments_on_channel_full() {
        let before = dropped_total();
        // Capacity 2: fill channel, then the 3rd send should drop
        let (exporter, _rx) = Exporter::start_test_with_capacity(2);
        exporter.send_traces(vec![0x0A]);
        exporter.send_traces(vec![0x0A]);
        // Channel is full now
        exporter.send_traces(vec![0x0A]);
        let delta = dropped_total() - before;
        assert!(delta >= 1, "expected at least 1 drop, got {}", delta);
    }

    #[tokio::test]
    async fn drop_counter_increments_for_logs_and_traces() {
        let before = dropped_total();
        let (exporter, _rx) = Exporter::start_test_with_capacity(1);
        exporter.send_traces(vec![0x0A]); // fills the channel
        exporter.send_traces(vec![0x0A]); // dropped
        exporter.send_logs(vec![0x0A]); // dropped
        let delta = dropped_total() - before;
        assert!(delta >= 2, "expected at least 2 drops, got {}", delta);
    }

    fn test_config(traces_url: Option<String>, logs_url: Option<String>) -> ExporterConfig {
        ExporterConfig {
            traces_url,
            logs_url,
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 512,
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: 4,
        }
    }

    #[tokio::test]
    async fn exporter_queues_and_flushes_without_panic() {
        let config = test_config(
            Some("http://127.0.0.1:1/v1/traces".to_string()),
            Some("http://127.0.0.1:1/v1/logs".to_string()),
        );
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.send_logs(vec![0x0A, 0x00]);

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_flush_completes() {
        let config = test_config(
            Some("http://127.0.0.1:1/v1/traces".to_string()),
            Some("http://127.0.0.1:1/v1/logs".to_string()),
        );
        let exporter = Exporter::start(config);

        tokio::time::timeout(Duration::from_secs(5), exporter.flush())
            .await
            .expect("flush should complete within timeout");

        exporter.shutdown().await;
    }

    async fn respond_with_status(listener: &tokio::net::TcpListener, status: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 65536];
        let _ = stream.read(&mut buf).await;
        let resp = format!(
            "HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            status
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn post_with_retry_succeeds_on_first_attempt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            respond_with_status(&listener, "200 OK").await;
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/v1/traces", addr);
        post_with_retry(&client, &url, Bytes::from_static(b"test")).await;
    }

    #[tokio::test]
    async fn post_with_retry_retries_on_500_then_succeeds() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            respond_with_status(&listener, "500 Internal Server Error").await;
            respond_with_status(&listener, "200 OK").await;
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/v1/traces", addr);
        post_with_retry(&client, &url, Bytes::from_static(b"test")).await;
    }

    #[tokio::test]
    async fn post_with_retry_gives_up_after_all_retries() {
        // Takes ~2.1s due to real retry delays (100ms + 400ms + 1600ms)
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            for _ in 0..3 {
                respond_with_status(&listener, "500 Internal Server Error").await;
            }
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/v1/traces", addr);
        post_with_retry(&client, &url, Bytes::from_static(b"test")).await;
        // Returns without panic after exhausting retries
    }

    #[tokio::test]
    async fn exporter_sends_to_correct_url_paths() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (path_tx, mut path_rx) = mpsc::channel::<String>(16);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let path_tx = path_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .to_string();
                    let _ = path_tx.send(path).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let config = test_config(
            Some(format!("http://{}/v1/traces", addr)),
            Some(format!("http://{}/v1/logs", addr)),
        );
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.send_logs(vec![0x0A, 0x00]);
        exporter.flush().await;

        let mut paths = Vec::new();
        while let Ok(Some(path)) =
            tokio::time::timeout(Duration::from_secs(5), path_rx.recv()).await
        {
            paths.push(path);
            if paths.len() >= 2 {
                break;
            }
        }

        assert!(
            paths.contains(&"/v1/traces".to_string()),
            "missing /v1/traces, got {:?}",
            paths
        );
        assert!(
            paths.contains(&"/v1/logs".to_string()),
            "missing /v1/logs, got {:?}",
            paths
        );

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_skips_logs_when_no_logs_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (path_tx, mut path_rx) = mpsc::channel::<String>(16);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let path_tx = path_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .to_string();
                    let _ = path_tx.send(path).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        // Only traces_url set, logs_url is None
        let config = test_config(Some(format!("http://{}/v1/traces", addr)), None);
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.send_logs(vec![0x0A, 0x00]); // should be silently dropped
        exporter.flush().await;

        let mut paths = Vec::new();
        while let Ok(Some(path)) =
            tokio::time::timeout(Duration::from_millis(500), path_rx.recv()).await
        {
            paths.push(path);
        }

        assert!(
            paths.contains(&"/v1/traces".to_string()),
            "expected /v1/traces, got {:?}",
            paths
        );
        assert!(
            !paths.contains(&"/v1/logs".to_string()),
            "should NOT have received /v1/logs, got {:?}",
            paths
        );

        exporter.shutdown().await;
    }

    #[test]
    fn exporter_config_with_batch_settings() {
        let config = ExporterConfig {
            traces_url: None,
            logs_url: None,
            channel_capacity: 16,
            metrics_url: None,
            batch_size: 100,
            flush_interval: Duration::from_millis(500),
            max_concurrent_exports: 2,
        };
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.flush_interval, Duration::from_millis(500));
        assert_eq!(config.max_concurrent_exports, 2);
    }

    #[tokio::test]
    async fn exporter_batches_traces_up_to_batch_size() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (body_tx, mut body_rx) = mpsc::channel::<Vec<u8>>(16);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body_tx = body_tx.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    buf.truncate(n);
                    // Extract body after \r\n\r\n
                    let request = &buf[..n];
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let body = request[pos + 4..].to_vec();
                        let _ = body_tx.send(body).await;
                    }
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 3,
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: 4,
        };
        let exporter = Exporter::start(config);

        // Send exactly batch_size items
        let payload = vec![0x0A, 0x00]; // minimal protobuf
        exporter.send_traces(payload.clone());
        exporter.send_traces(payload.clone());
        exporter.send_traces(payload.clone());
        exporter.flush().await;

        // Should receive exactly 1 HTTP POST containing all 3 concatenated
        let body = tokio::time::timeout(Duration::from_secs(5), body_rx.recv())
            .await
            .expect("timeout waiting for POST")
            .expect("channel closed");

        // 3 * 2 bytes = 6 bytes total (concatenated payloads)
        assert_eq!(body.len(), 6);

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_flushes_on_interval() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let done_tx = done_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = done_tx.send(()).await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 100, // large batch size — won't trigger batch flush
            flush_interval: Duration::from_millis(200), // short interval
            max_concurrent_exports: 4,
        };
        let exporter = Exporter::start(config);

        // Send 1 item — won't reach batch_size, must flush on interval
        exporter.send_traces(vec![0x0A, 0x00]);

        // Should arrive within ~500ms (interval + processing)
        let result = tokio::time::timeout(Duration::from_millis(1000), done_rx.recv()).await;
        assert!(result.is_ok(), "data should arrive via interval flush");

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_explicit_flush_drains_batch() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let done_tx = done_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = done_tx.send(()).await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 100, // large — won't trigger automatically
            flush_interval: Duration::from_secs(60), // long — won't trigger on time
            max_concurrent_exports: 4,
        };
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.flush().await;

        // Should have been sent by now
        let result = tokio::time::timeout(Duration::from_millis(500), done_rx.recv()).await;
        assert!(result.is_ok(), "flush should drain pending batch");

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_shutdown_drains_remaining_batch() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (done_tx, mut done_rx) = mpsc::channel::<()>(1);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let done_tx = done_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = done_tx.send(()).await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 100,
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: 4,
        };
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.shutdown().await;

        // Shutdown should have drained the batch
        let result = tokio::time::timeout(Duration::from_millis(500), done_rx.recv()).await;
        assert!(result.is_ok(), "shutdown should drain remaining batch");
    }

    #[tokio::test]
    async fn exporter_batches_traces_and_logs_independently() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (path_tx, mut path_rx) = mpsc::channel::<String>(16);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let path_tx = path_tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .to_string();
                    let _ = path_tx.send(path).await;
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: Some(format!("http://{}/v1/logs", addr)),
            metrics_url: None,
            channel_capacity: 16,
            batch_size: 2,
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: 4,
        };
        let exporter = Exporter::start(config);

        // Fill trace batch (batch_size = 2)
        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.send_traces(vec![0x0A, 0x00]);
        // Fill log batch (batch_size = 2)
        exporter.send_logs(vec![0x0A, 0x00]);
        exporter.send_logs(vec![0x0A, 0x00]);
        exporter.flush().await;

        let mut paths = Vec::new();
        while let Ok(Some(path)) =
            tokio::time::timeout(Duration::from_secs(5), path_rx.recv()).await
        {
            paths.push(path);
            if paths.len() >= 2 {
                break;
            }
        }

        assert!(
            paths.contains(&"/v1/traces".to_string()),
            "missing /v1/traces, got {:?}",
            paths
        );
        assert!(
            paths.contains(&"/v1/logs".to_string()),
            "missing /v1/logs, got {:?}",
            paths
        );

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_sends_concurrently_not_sequentially() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomOrd};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let concurrent_c = concurrent.clone();
        let max_concurrent_c = max_concurrent.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let conc = concurrent_c.clone();
                let max_conc = max_concurrent_c.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf).await;
                    let current = conc.fetch_add(1, AtomOrd::SeqCst) + 1;
                    max_conc.fetch_max(current, AtomOrd::SeqCst);
                    // Hold the connection open for a bit to allow overlap
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    conc.fetch_sub(1, AtomOrd::SeqCst);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 64,
            batch_size: 1, // flush on every message — creates many concurrent posts
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: 8,
        };
        let exporter = Exporter::start(config);

        // Send many items rapidly to trigger concurrent exports
        for _ in 0..8 {
            exporter.send_traces(vec![0x0A, 0x00]);
        }
        exporter.flush().await;

        assert!(
            max_concurrent.load(AtomOrd::SeqCst) > 1,
            "expected concurrent exports > 1, got {}",
            max_concurrent.load(AtomOrd::SeqCst)
        );

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_limits_concurrent_exports() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomOrd};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let max_concurrent = Arc::new(AtomicUsize::new(0));
        let concurrent = Arc::new(AtomicUsize::new(0));

        let concurrent_c = concurrent.clone();
        let max_concurrent_c = max_concurrent.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let conc = concurrent_c.clone();
                let max_conc = max_concurrent_c.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 65536];
                    let _ = stream.read(&mut buf).await;
                    let current = conc.fetch_add(1, AtomOrd::SeqCst) + 1;
                    max_conc.fetch_max(current, AtomOrd::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    conc.fetch_sub(1, AtomOrd::SeqCst);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let max_exports = 2;
        let config = ExporterConfig {
            traces_url: Some(format!("http://{}/v1/traces", addr)),
            logs_url: None,
            metrics_url: None,
            channel_capacity: 64,
            batch_size: 1,
            flush_interval: Duration::from_secs(60),
            max_concurrent_exports: max_exports,
        };
        let exporter = Exporter::start(config);

        for _ in 0..8 {
            exporter.send_traces(vec![0x0A, 0x00]);
        }
        exporter.flush().await;

        assert!(
            max_concurrent.load(AtomOrd::SeqCst) <= max_exports,
            "expected max concurrent <= {}, got {}",
            max_exports,
            max_concurrent.load(AtomOrd::SeqCst)
        );

        exporter.shutdown().await;
    }
}

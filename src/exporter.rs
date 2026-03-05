use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;

/// Message types sent to the exporter background task.
pub(crate) enum ExportMessage {
    Traces(Bytes),
    Logs(Bytes),
    Flush(tokio::sync::oneshot::Sender<()>),
    Shutdown,
}

/// Configuration for the exporter.
pub(crate) struct ExporterConfig {
    pub endpoint: String,
    pub channel_capacity: usize,
}

/// Handle to the exporter background task.
#[derive(Clone)]
pub(crate) struct Exporter {
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
        tokio::spawn(exporter_loop(rx, client, config.endpoint));
        Self { tx }
    }

    /// Send encoded trace data to the exporter.
    pub fn send_traces(&self, data: Vec<u8>) {
        let _ = self.tx.try_send(ExportMessage::Traces(Bytes::from(data)));
    }

    /// Send encoded log data to the exporter.
    pub fn send_logs(&self, data: Vec<u8>) {
        let _ = self.tx.try_send(ExportMessage::Logs(Bytes::from(data)));
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

async fn exporter_loop(
    mut rx: mpsc::Receiver<ExportMessage>,
    client: reqwest::Client,
    endpoint: String,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            ExportMessage::Traces(data) => {
                let url = format!("{}/v1/traces", endpoint);
                post_with_retry(&client, &url, data).await;
            }
            ExportMessage::Logs(data) => {
                let url = format!("{}/v1/logs", endpoint);
                post_with_retry(&client, &url, data).await;
            }
            ExportMessage::Flush(done) => {
                // All prior messages processed sequentially; signal completion.
                let _ = done.send(());
            }
            ExportMessage::Shutdown => {
                break;
            }
        }
    }
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

    #[tokio::test]
    async fn exporter_queues_and_flushes_without_panic() {
        let config = ExporterConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            channel_capacity: 16,
        };
        let exporter = Exporter::start(config);

        exporter.send_traces(vec![0x0A, 0x00]);
        exporter.send_logs(vec![0x0A, 0x00]);

        exporter.shutdown().await;
    }

    #[tokio::test]
    async fn exporter_flush_completes() {
        let config = ExporterConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            channel_capacity: 16,
        };
        let exporter = Exporter::start(config);

        tokio::time::timeout(Duration::from_secs(5), exporter.flush())
            .await
            .expect("flush should complete within timeout");

        exporter.shutdown().await;
    }
}

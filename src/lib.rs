pub(crate) mod exporter;
pub mod metrics;
pub(crate) mod otlp_layer;
pub(crate) mod otlp_log;
pub(crate) mod otlp_metrics;
pub(crate) mod otlp_trace;
pub(crate) mod proto;
#[cfg(feature = "tower")]
pub mod tower;
pub mod trace_id;
pub mod constants;
pub(crate) mod use_metrics;

#[cfg(feature = "tower")]
pub use tower::propagation::PropagationLayer;
#[cfg(feature = "tower")]
pub use tower::request::CfRequestIdLayer;

pub use metrics::{counter, gauge, Counter, Gauge};

use std::time::Duration;

use exporter::{Exporter, ExporterConfig};
use otlp_layer::OtlpLayer;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Configuration for the telemetry stack.
pub struct TelemetryConfig {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub environment: &'static str,
    /// OTLP HTTP endpoint for traces (e.g. "http://jaeger:4318").
    /// If None, trace export is disabled.
    pub otlp_traces_endpoint: Option<&'static str>,
    /// OTLP HTTP endpoint for logs (e.g. "http://vector:4318").
    /// If None, log export is disabled. Can differ from traces endpoint.
    pub otlp_logs_endpoint: Option<&'static str>,
    /// OTLP HTTP endpoint for metrics (e.g. "http://vector:4318").
    /// If None, metrics export is disabled.
    pub otlp_metrics_endpoint: Option<&'static str>,
    /// Whether to emit JSON-formatted logs to stderr.
    pub log_to_stderr: bool,
    /// Polling interval for USE metrics (cpu, memory) from `/proc/self/stat`.
    /// If None, USE metrics collection is disabled.
    /// Only active on Linux; no-op on other platforms.
    pub use_metrics_interval: Option<Duration>,
    /// How often to flush aggregated metrics to the OTLP endpoint.
    /// Defaults to 10 seconds if None.
    pub metrics_flush_interval: Option<Duration>,
}

/// Guard that flushes pending telemetry on drop.
///
/// Hold this in your main function to ensure all spans are exported before shutdown.
pub struct TelemetryGuard {
    exporter: Option<Exporter>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(ref exporter) = self.exporter {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            if let Ok(rt) = rt {
                rt.block_on(async {
                    exporter.flush().await;
                    exporter.shutdown().await;
                });
            }
        }
    }
}

/// Initialize the telemetry stack.
///
/// Sets up:
/// 1. `fmt::Layer` with JSON output to stderr (if `log_to_stderr` is true)
/// 2. `OtlpLayer` connected to an HTTP exporter (if either OTLP endpoint is Some)
/// 3. `EnvFilter` from `RUST_LOG` (default: `info,tower_http=info`)
/// 4. Metrics aggregation task (if `otlp_metrics_endpoint` is Some)
///
/// Returns a guard that flushes pending telemetry on drop.
pub fn init(config: TelemetryConfig) -> TelemetryGuard {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));

    let fmt_layer = if config.log_to_stderr {
        Some(
            fmt::layer()
                .json()
                .with_target(true)
                .with_current_span(true)
                .with_span_list(false)
                .with_writer(std::io::stderr),
        )
    } else {
        None
    };

    let export_traces = config.otlp_traces_endpoint.is_some();
    let export_logs = config.otlp_logs_endpoint.is_some();
    let export_metrics = config.otlp_metrics_endpoint.is_some();

    let metrics_url = config
        .otlp_metrics_endpoint
        .map(|ep| format!("{}/v1/metrics", ep));

    let (otlp_layer, exporter) = if export_traces || export_logs || export_metrics {
        let traces_url = config
            .otlp_traces_endpoint
            .map(|ep| format!("{}/v1/traces", ep));
        let logs_url = config
            .otlp_logs_endpoint
            .map(|ep| format!("{}/v1/logs", ep));
        let exp = Exporter::start(ExporterConfig {
            traces_url,
            logs_url,
            metrics_url: metrics_url.clone(),
            channel_capacity: 1024,
            batch_size: 512,
            flush_interval: Duration::from_secs(1),
            max_concurrent_exports: 4,
        });
        let layer = if export_traces || export_logs {
            Some(OtlpLayer::new(
                exp.clone(),
                config.service_name,
                config.service_version,
                config.environment,
                export_traces,
                export_logs,
            ))
        } else {
            None
        };
        (layer, Some(exp))
    } else {
        (None, None)
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otlp_layer)
        .init();

    tracing::info!(
        service.name = config.service_name,
        service.version = config.service_version,
        environment = config.environment,
        "telemetry initialized"
    );

    if let Some(interval) = config.use_metrics_interval {
        use_metrics::start(interval);
    }

    // Spawn metrics aggregation task
    if let Some(ref exporter) = exporter {
        if metrics_url.is_some() {
            let flush_interval = config
                .metrics_flush_interval
                .unwrap_or(Duration::from_secs(10));
            let exporter = exporter.clone();
            let service_name = config.service_name;
            let service_version = config.service_version;
            let environment = config.environment;
            tokio::spawn(async move {
                metrics_aggregation_loop(
                    exporter,
                    flush_interval,
                    service_name,
                    service_version,
                    environment,
                )
                .await;
            });
        }
    }

    TelemetryGuard { exporter }
}

/// Background task that periodically collects and exports aggregated metrics.
async fn metrics_aggregation_loop(
    exporter: Exporter,
    flush_interval: Duration,
    service_name: &'static str,
    service_version: &'static str,
    environment: &'static str,
) {
    use crate::otlp_metrics::encode_export_metrics_request;
    use crate::otlp_trace::{AnyValue, KeyValue};

    let resource_attrs = vec![
        KeyValue {
            key: "service.name".to_string(),
            value: AnyValue::String(service_name.to_string()),
        },
        KeyValue {
            key: "service.version".to_string(),
            value: AnyValue::String(service_version.to_string()),
        },
        KeyValue {
            key: "deployment.environment".to_string(),
            value: AnyValue::String(environment.to_string()),
        },
    ];

    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let mut interval = tokio::time::interval(flush_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Consume the first immediate tick
    interval.tick().await;

    loop {
        interval.tick().await;

        let registry = metrics::global_registry();
        let snapshots = registry.collect();
        if snapshots.is_empty() {
            continue;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let data = encode_export_metrics_request(
            &resource_attrs,
            "ro11y",
            service_version,
            &snapshots,
            start_time,
            now,
        );

        exporter.send_metrics(data);
    }
}

/// Return the total number of telemetry messages dropped due to a full channel.
pub fn telemetry_dropped_total() -> u64 {
    exporter::dropped_total()
}

/// Convenience: create a `CfRequestIdLayer` for incoming requests.
#[cfg(feature = "tower")]
pub fn request_layer() -> CfRequestIdLayer {
    CfRequestIdLayer
}

/// Convenience: create a `PropagationLayer` for outgoing requests.
#[cfg(feature = "tower")]
pub fn propagation_layer() -> PropagationLayer {
    PropagationLayer
}

#[cfg(feature = "_bench")]
#[doc(hidden)]
pub mod bench {
    pub use crate::exporter::{Exporter, ExporterConfig};
    pub use crate::metrics::{
        counter, gauge, global_registry, Counter, Gauge, MetricSnapshot, MetricsRegistry,
    };
    pub use crate::otlp_log::{encode_export_logs_request, LogData, SeverityNumber};
    pub use crate::otlp_metrics::encode_export_metrics_request;
    pub use crate::otlp_trace::{
        encode_export_trace_request, encode_key_value, encode_resource, AnyValue, KeyValue,
        SpanData, SpanKind, SpanStatus, StatusCode,
    };
    pub use crate::proto::encode_message_field_in_place;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_with_none_endpoint_does_not_panic() {
        let _config = TelemetryConfig {
            service_name: "test-service",
            service_version: "0.0.1",
            environment: "test",
            otlp_traces_endpoint: None,
            otlp_logs_endpoint: None,
            otlp_metrics_endpoint: None,
            log_to_stderr: false,
            use_metrics_interval: None,
            metrics_flush_interval: None,
        };
    }

    #[cfg(feature = "tower")]
    #[test]
    fn request_layer_constructs() {
        let _layer = request_layer();
    }

    #[cfg(feature = "tower")]
    #[test]
    fn propagation_layer_constructs() {
        let _layer = propagation_layer();
    }

    #[test]
    fn telemetry_dropped_total_is_callable() {
        let _count = telemetry_dropped_total();
    }
}

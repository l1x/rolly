pub(crate) mod exporter;
pub(crate) mod otlp_layer;
pub(crate) mod otlp_log;
pub(crate) mod otlp_trace;
pub(crate) mod proto;
pub mod tower;
pub mod trace_id;
pub(crate) mod use_metrics;

pub use tower::propagation::PropagationLayer;
pub use tower::request::CfRequestIdLayer;

use std::time::Duration;

use exporter::{Exporter, ExporterConfig};
use otlp_layer::OtlpLayer;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Configuration for the telemetry stack.
pub struct TelemetryConfig {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub environment: &'static str,
    /// OTLP HTTP endpoint (e.g. "http://vector:4318").
    /// If None, only JSON stderr output is enabled (local dev mode).
    pub otlp_endpoint: Option<&'static str>,
    /// Polling interval for USE metrics (cpu, memory) from `/proc/self/stat`.
    /// If None, USE metrics collection is disabled.
    /// Only active on Linux; no-op on other platforms.
    pub use_metrics_interval: Option<Duration>,
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
/// 1. `fmt::Layer` with JSON output to stderr (CloudWatch fallback, always on)
/// 2. `OtlpLayer` connected to an HTTP exporter (only if `otlp_endpoint` is Some)
/// 3. `EnvFilter` from `RUST_LOG` (default: `info,tower_http=info`)
///
/// Returns a guard that flushes pending telemetry on drop.
pub fn init(config: TelemetryConfig) -> TelemetryGuard {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));

    let fmt_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_current_span(true)
        .with_span_list(false)
        .with_writer(std::io::stderr);

    let (otlp_layer, exporter) = match config.otlp_endpoint {
        Some(endpoint) => {
            let exp = Exporter::start(ExporterConfig {
                endpoint: endpoint.to_string(),
                channel_capacity: 1024,
            });
            let layer = OtlpLayer::new(
                exp.clone(),
                config.service_name,
                config.service_version,
                config.environment,
            );
            (Some(layer), Some(exp))
        }
        None => (None, None),
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

    TelemetryGuard { exporter }
}

/// Convenience: create a `CfRequestIdLayer` for incoming requests.
pub fn request_layer() -> CfRequestIdLayer {
    CfRequestIdLayer
}

/// Convenience: create a `PropagationLayer` for outgoing requests.
pub fn propagation_layer() -> PropagationLayer {
    PropagationLayer
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
            otlp_endpoint: None,
            use_metrics_interval: None,
        };
    }

    #[test]
    fn request_layer_constructs() {
        let _layer = request_layer();
    }

    #[test]
    fn propagation_layer_constructs() {
        let _layer = propagation_layer();
    }
}

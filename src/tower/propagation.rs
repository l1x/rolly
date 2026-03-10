use std::task::{Context, Poll};

use http::Request;
use tower::{Layer, Service};

use crate::trace_id::hex_encode;

/// Tower Layer that injects W3C `traceparent` header into outgoing requests.
///
/// Reads trace_id and span_id from the current span's extensions (set by OtlpLayer)
/// and formats them as `traceparent: 00-{trace_id}-{span_id}-01`.
#[derive(Clone, Debug)]
pub struct PropagationLayer;

#[derive(Clone, Debug)]
pub struct PropagationService<S> {
    inner: S,
}

impl<S> Layer<S> for PropagationLayer {
    type Service = PropagationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PropagationService { inner }
    }
}

impl<S, ReqBody> Service<Request<ReqBody>> for PropagationService<S>
where
    S: Service<Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        // Read trace_id/span_id from the OtlpLayer's SpanFields extension.
        // This walks the subscriber's span registry to find our extension data.
        let mut trace_id_hex = None;
        let mut span_id_hex = None;

        tracing::Span::current().with_subscriber(|(id, dispatch)| {
            use tracing_subscriber::registry::LookupSpan;
            if let Some(registry) = dispatch.downcast_ref::<tracing_subscriber::Registry>() {
                if let Some(span_ref) = registry.span(id) {
                    // Walk up the span tree to find the nearest span with SpanFields.
                    let ext = span_ref.extensions();
                    if let Some(fields) = ext.get::<crate::otlp_layer::SpanFields>() {
                        trace_id_hex = Some(hex_encode(&fields.trace_id));
                        span_id_hex = Some(hex_encode(&fields.span_id));
                    } else {
                        // Check parent spans
                        for ancestor in span_ref.scope().skip(1) {
                            let ext = ancestor.extensions();
                            if let Some(fields) = ext.get::<crate::otlp_layer::SpanFields>() {
                                trace_id_hex = Some(hex_encode(&fields.trace_id));
                                span_id_hex = Some(hex_encode(&fields.span_id));
                                break;
                            }
                        }
                    }
                }
            }
        });

        if let (Some(tid), Some(sid)) = (trace_id_hex, span_id_hex) {
            let traceparent = format!("00-{}-{}-01", tid, sid);
            if let Ok(val) = http::HeaderValue::from_str(&traceparent) {
                req.headers_mut().insert("traceparent", val);
            }
        }

        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagation_layer_constructs() {
        let _layer = PropagationLayer;
    }

    #[test]
    fn traceparent_format() {
        let trace_id = [1u8; 16];
        let span_id = [2u8; 8];
        let tp = format!("00-{}-{}-01", hex_encode(&trace_id), hex_encode(&span_id));
        assert_eq!(
            tp,
            "00-01010101010101010101010101010101-0202020202020202-01"
        );
    }
}
